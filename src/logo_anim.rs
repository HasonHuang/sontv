//! 台标动效：解码静态图、逐帧生成、编码动画 WebP。
//!
//! 目标 v0.8.2 使用 10 帧无损动画 WebP（F-012）。PNG 与 WebP 由纯 Rust
//! 解码；其他白名单格式仍可回退到 `ffmpeg`。动效输出覆盖原文件，并在
//! `.orig/` 保留首次生成前的静态原图。

use std::{fs, io::Cursor, path::Path, process::Command};

use anyhow::{bail, Context, Result};
use image_webp::{ColorType, WebPDecoder, WebPEncoder};

/// 动画固定帧数。
const FRAMES: usize = 10;
/// 前 9 帧时长。
const FRAME_MS: u64 = 80;
/// 末帧时长。
const LAST_FRAME_MS: u64 = 1_500;
/// 总时长。
const TOTAL_MS: u64 = FRAME_MS * 9 + LAST_FRAME_MS;
/// 单级增亮量。
const LEVEL: f32 = 0.11;

/// 动效生成结果。
#[derive(Debug, Clone)]
pub struct Report {
    pub frames: usize,
    pub width: u32,
    pub height: u32,
    pub total_ms: u64,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub dirty_ratio: f64,
}

/// 生成动画 WebP 并覆盖原台标。
///
/// `frame_ms` 与 `quality` 保留在接口中，目标 v0.8.2 的动效编码固定为
/// 80/1500 毫秒且无损，不读取这两个值。
pub fn animate(
    cache_dir: &str,
    file: &str,
    effect: &str,
    quality: &str,
    max_size: u32,
    _frame_ms: u64,
    default_quality: u8,
) -> Result<Report> {
    let file = crate::logo::sanitize(file).context("文件名不合法")?;
    let ext = file
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .unwrap_or_default();
    if !crate::logo::ALLOWED_EXTS.contains(&ext.as_str()) {
        bail!("不支持的台标扩展名");
    }
    let path = crate::logo::dir(cache_dir).join(&file);
    if !path.is_file() {
        bail!("文件不存在");
    }
    if crate::logo::is_animated_content(&path) {
        bail!("源图已经是动画,请先用静态图生成");
    }
    let quality = if quality.trim().is_empty() {
        default_quality
    } else {
        quality
            .trim()
            .parse::<u8>()
            .context("quality 必须是 1..100 的字符串")?
    };
    if !(1..=100).contains(&quality) {
        bail!("quality 必须在 1..100");
    }
    let _ = quality;

    let max_size = max_size.clamp(16, 2048);
    let source = decode(&path)?;
    let (width, height) = scaled_size(source.width, source.height, max_size);
    let source = if width == source.width && height == source.height {
        source
    } else {
        Image {
            width,
            height,
            pixels: resize(&source, width, height),
        }
    };

    let frames = render_frames(&source, effect)?;
    let rects = frame_rects(&frames, width, height);
    let dirty_ratio = dirty_ratio(&rects, width, height);
    let output = encode_animated_webp(&frames, &rects, width, height)?;

    let bytes_before = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let original = crate::logo::orig_dir(cache_dir).join(&file);
    if !original.is_file() {
        fs::create_dir_all(crate::logo::orig_dir(cache_dir)).context("建原图目录")?;
        fs::copy(&path, &original).context("备份静态原图")?;
    }
    fs::write(&path, &output).with_context(|| format!("写 {}", path.display()))?;
    Ok(Report {
        frames: frames.len(),
        width,
        height,
        total_ms: TOTAL_MS,
        bytes_before,
        bytes_after: output.len() as u64,
        dirty_ratio,
    })
}

#[derive(Debug, Clone)]
struct Image {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
}

fn scaled_size(width: u32, height: u32, max_size: u32) -> (u32, u32) {
    let longest = width.max(height);
    if longest <= max_size {
        return (width.max(1), height.max(1));
    }
    let scale = max_size as f64 / longest as f64;
    (
        ((width as f64 * scale).round() as u32).max(1),
        ((height as f64 * scale).round() as u32).max(1),
    )
}

fn decode(path: &Path) -> Result<Image> {
    let data = fs::read(path).with_context(|| format!("读 {}", path.display()))?;
    if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        return decode_png(&data);
    }
    if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        return decode_webp(&data);
    }
    decode_ffmpeg(path)
}

fn decode_png(data: &[u8]) -> Result<Image> {
    let mut decoder = png::Decoder::new(Cursor::new(data));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().context("解析 PNG")?;
    let (width, height) = (reader.info().width, reader.info().height);
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).context("解码 PNG")?;
    let (color, bit_depth) = reader.output_color_type();
    if bit_depth != png::BitDepth::Eight {
        bail!("PNG 位深不支持");
    }
    let pixels = match color {
        png::ColorType::Grayscale => {
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for value in buffer.iter().take(info.buffer_size()) {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
            rgba
        }
        png::ColorType::GrayscaleAlpha => {
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for pixel in buffer[..info.buffer_size()].as_chunks::<2>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]]);
            }
            rgba
        }
        png::ColorType::Rgb => {
            let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
            for pixel in buffer[..info.buffer_size()].as_chunks::<3>().0 {
                rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
            }
            rgba
        }
        png::ColorType::Rgba => buffer[..info.buffer_size()].to_vec(),
        png::ColorType::Indexed => bail!("PNG 调色板未展开"),
    };
    let expected = width as usize * height as usize * 4;
    if pixels.len() != expected {
        bail!("PNG 输出尺寸不符");
    }
    Ok(Image {
        width,
        height,
        pixels,
    })
}

fn decode_webp(data: &[u8]) -> Result<Image> {
    let mut decoder = WebPDecoder::new(Cursor::new(data)).context("解析 WebP")?;
    let (width, height) = decoder.dimensions();
    let alpha = decoder.has_alpha();
    let size = decoder.output_buffer_size().context("WebP 尺寸溢出")?;
    let mut buffer = vec![0; size];
    decoder.read_image(&mut buffer).context("解码 WebP")?;
    let pixels = if alpha {
        buffer
    } else {
        let mut rgba = Vec::with_capacity(width as usize * height as usize * 4);
        for pixel in buffer.as_chunks::<3>().0 {
            rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255]);
        }
        rgba
    };
    Ok(Image {
        width,
        height,
        pixels,
    })
}

fn decode_ffmpeg(path: &Path) -> Result<Image> {
    let (width, height) = probe_ffmpeg(path)?;
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba", "-"])
        .output()
        .context("执行 ffmpeg 失败")?;
    if !output.status.success() {
        bail!(
            "ffmpeg 解码失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let expected = width as usize * height as usize * 4;
    if output.stdout.len() != expected {
        bail!("ffmpeg 输出尺寸不符: {} != {expected}", output.stdout.len());
    }
    Ok(Image {
        width,
        height,
        pixels: output.stdout,
    })
}

fn probe_ffmpeg(path: &Path) -> Result<(u32, u32)> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height",
            "-of",
            "csv=s=x:p=0",
        ])
        .arg(path)
        .output()
        .context("执行 ffprobe 失败")?;
    if !output.status.success() {
        bail!(
            "ffprobe 读取失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (width, height) = text.trim().split_once('x').context("ffprobe 未返回尺寸")?;
    Ok((width.parse()?, height.parse()?))
}

/// 双线性缩放；采样点按左上角对齐，避免半像素偏移。
fn resize(source: &Image, width: u32, height: u32) -> Vec<u8> {
    let sw = source.width as usize;
    let sh = source.height as usize;
    let dw = width as usize;
    let dh = height as usize;
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let sy = y as f32 * sh as f32 / dh as f32;
        let y0 = (sy.floor() as usize).min(sh.saturating_sub(1));
        let y1 = (y0 + 1).min(sh - 1);
        let fy = sy - y0 as f32;
        for x in 0..dw {
            let sx = x as f32 * sw as f32 / dw as f32;
            let x0 = (sx.floor() as usize).min(sw.saturating_sub(1));
            let x1 = (x0 + 1).min(sw - 1);
            let fx = sx - x0 as f32;
            let offset = (y * dw + x) * 4;
            for channel in 0..4 {
                let a = source.pixels[(y0 * sw + x0) * 4 + channel] as f32;
                let b = source.pixels[(y0 * sw + x1) * 4 + channel] as f32;
                let c = source.pixels[(y1 * sw + x0) * 4 + channel] as f32;
                let d = source.pixels[(y1 * sw + x1) * 4 + channel] as f32;
                let top = a + (b - a) * fx;
                let bottom = c + (d - c) * fx;
                out[offset + channel] = (top + (bottom - top) * fy).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

fn render_frames(source: &Image, effect: &str) -> Result<Vec<Vec<u8>>> {
    let w = source.width as usize;
    let h = source.height as usize;
    match effect {
        "diagonal" | "vertical" | "bounce" | "spot" | "glow" | "breathe" => {}
        other => bail!("未知动效: {other}"),
    }
    let mut frames = Vec::with_capacity(FRAMES);
    for frame in 0..FRAMES {
        let mut pixels = source.pixels.clone();
        match effect {
            "diagonal" => diagonal(&mut pixels, w, h, frame),
            "vertical" => vertical(&mut pixels, w, h, frame),
            "bounce" => bounce(&mut pixels, w, h, frame),
            "spot" => spot(&mut pixels, w, h, frame),
            "glow" => glow(&mut pixels, w, h, frame),
            "breathe" => breathe(&mut pixels, frame),
            _ => unreachable!(),
        }
        frames.push(pixels);
    }
    Ok(frames)
}

/// 斜向扫光：光带方向为 `x + y/4`，宽度 17 像素。
fn diagonal(pixels: &mut [u8], w: usize, h: usize, frame: usize) {
    if frame == FRAMES - 1 {
        return;
    }
    let max_pos = (w.saturating_sub(1)) as f32 + (h.saturating_sub(1)) as f32 * 0.25;
    let center = (frame as f32 / (FRAMES - 1) as f32 * (max_pos + 1.0)).round();
    sweep(
        pixels,
        w,
        h,
        |x, y| x as f32 + y as f32 * 0.25,
        center,
        &DIAGONAL_PROFILE,
    );
}

/// 竖向扫光：第 1 帧从顶部进入，第 8 帧从底部离开。
fn vertical(pixels: &mut [u8], w: usize, h: usize, frame: usize) {
    if frame == 0 || frame == FRAMES - 1 {
        return;
    }
    let center = ((frame - 1) as f32 * h as f32 / 7.0).round();
    sweep(pixels, w, h, |_, y| y as f32, center, &VERTICAL_PROFILE);
}

/// 来回扫光：斜向光带往返一次。
fn bounce(pixels: &mut [u8], w: usize, h: usize, frame: usize) {
    if frame == FRAMES - 1 {
        return;
    }
    let max_pos = (w.saturating_sub(1)) as f32 + (h.saturating_sub(1)) as f32 * 0.25;
    let phase = match frame {
        0 => 0.0,
        1 => 2.0 / 9.0,
        2 => 4.0 / 9.0,
        3 => 6.0 / 9.0,
        4 | 5 => 8.0 / 9.0,
        6 => 6.0 / 9.0,
        7 => 4.0 / 9.0,
        _ => 2.0 / 9.0,
    };
    sweep(
        pixels,
        w,
        h,
        |x, y| x as f32 + y as f32 * 0.25,
        (phase * (max_pos + 1.0)).round(),
        &DIAGONAL_PROFILE,
    );
}

fn sweep<F>(pixels: &mut [u8], w: usize, h: usize, pos: F, center: f32, profile: &[u8])
where
    F: Fn(usize, usize) -> f32,
{
    let half = profile.len() as f32 / 2.0;
    for y in 0..h {
        for x in 0..w {
            let delta = pos(x, y) - center;
            if delta < -half || delta >= half {
                continue;
            }
            let index = (delta + half).floor() as usize;
            let Some(level) = profile.get(index).copied() else {
                continue;
            };
            add_white(&mut pixels[(y * w + x) * 4..][..4], level);
        }
    }
}

/// 斜向光带级别。
const DIAGONAL_PROFILE: [u8; 17] = [1, 2, 3, 4, 4, 5, 5, 5, 5, 5, 5, 4, 4, 3, 3, 2, 1];
/// 竖向光带级别。
const VERTICAL_PROFILE: [u8; 17] = [1, 2, 3, 3, 4, 4, 5, 5, 5, 5, 5, 5, 4, 4, 3, 2, 1];
/// 光点掠过：椭圆光斑从左上向右中移动。
fn spot(pixels: &mut [u8], w: usize, h: usize, frame: usize) {
    if frame == FRAMES - 1 {
        return;
    }
    let step = (frame as f32 - 1.0) / 7.0;
    let cx = step * w as f32;
    let cy = h as f32 * (0.35 + 0.30 * step);
    let radius = (w.max(h) as f32 * 0.15).max(3.0);
    for y in 0..h {
        for x in 0..w {
            let dx = (x as f32 - cx) / radius;
            let dy = (y as f32 - cy) / radius;
            let distance_squared = dx * dx + dy * dy;
            if distance_squared >= 1.0 {
                continue;
            }
            let level = (5.0 * (1.0 - distance_squared)).round().clamp(1.0, 5.0) as u8;
            add_white(&mut pixels[(y * w + x) * 4..][..4], level);
        }
    }
}

/// 边缘辉光：按帧强度在图形边缘向外扩散 2–3 像素。
fn glow(pixels: &mut [u8], w: usize, h: usize, frame: usize) {
    let base = [0u8, 1, 2, 3, 3, 3, 3, 2, 1, 0][frame.min(FRAMES - 1)];
    if base == 0 {
        return;
    }
    let rings = if base <= 1 { 2 } else { 3 };
    let distances = edge_distances(pixels, w, h, rings);
    for (index, distance) in distances.iter().enumerate() {
        if let Some(distance) = distance {
            let level = base.saturating_sub(*distance).max(1);
            add_white(&mut pixels[index * 4..][..4], level);
        }
    }
}

fn edge_distances(pixels: &[u8], w: usize, h: usize, rings: usize) -> Vec<Option<u8>> {
    let mut edge = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let index = y * w + x;
            if pixels[index * 4 + 3] == 0 {
                continue;
            }
            if x == 0
                || y == 0
                || x + 1 == w
                || y + 1 == h
                || pixels[index * 4 + 3 - 4] == 0
                || pixels[index * 4 + 3 + 4] == 0
                || pixels[index * 4 + 3 - w * 4] == 0
                || pixels[index * 4 + 3 + w * 4] == 0
                || color_edge(pixels, w, x, y)
            {
                edge[index] = true;
            }
        }
    }
    let mut distances = vec![None; w * h];
    let mut frontier: Vec<usize> = edge
        .iter()
        .enumerate()
        .filter_map(|(index, value)| (*value && pixels[index * 4 + 3] > 0).then_some(index))
        .collect();
    for distance in 0..rings {
        let mut next = Vec::new();
        for index in frontier {
            if distances[index].is_some() {
                continue;
            }
            distances[index] = Some(distance as u8);
            let x = index % w;
            let y = index / w;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let nx = x as i32 + dx;
                    let ny = y as i32 + dy;
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let neighbor = ny as usize * w + nx as usize;
                    if distances[neighbor].is_none() && pixels[neighbor * 4 + 3] > 0 {
                        next.push(neighbor);
                    }
                }
            }
        }
        frontier = next;
    }
    distances
}

fn color_edge(pixels: &[u8], w: usize, x: usize, y: usize) -> bool {
    let index = (y * w + x) * 4;
    let mut gradient = 0i32;
    if x + 1 < w {
        let next = index + 4;
        for channel in 0..3 {
            gradient = gradient
                .max((pixels[index + channel] as i32 - pixels[next + channel] as i32).abs());
        }
    }
    if index + w * 4 + 3 < pixels.len() {
        let next = index + w * 4;
        for channel in 0..3 {
            gradient = gradient
                .max((pixels[index + channel] as i32 - pixels[next + channel] as i32).abs());
        }
    }
    gradient > 40
}

/// 呼吸明暗：对全部不透明像素增亮。
fn breathe(pixels: &mut [u8], frame: usize) {
    let level = [0u8, 2, 3, 4, 5, 5, 4, 3, 2, 0][frame.min(FRAMES - 1)];
    if level == 0 {
        return;
    }
    for pixel in pixels.as_chunks_mut::<4>().0 {
        add_white(pixel, level);
    }
}

fn add_white(pixel: &mut [u8], level: u8) {
    if level == 0 || pixel[3] == 0 {
        return;
    }
    let amount = (level.min(5) as f32 * LEVEL).clamp(0.0, 1.0);
    for channel in &mut pixel[..3] {
        *channel = (*channel as f32 + (255.0 - *channel as f32) * amount)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

fn frame_rects(frames: &[Vec<u8>], width: u32, height: u32) -> Vec<Rect> {
    let mut rects = Vec::with_capacity(frames.len());
    for (index, frame) in frames.iter().enumerate() {
        if index == 0 {
            rects.push(Rect {
                x: 0,
                y: 0,
                width,
                height,
            });
            continue;
        }
        let previous = &frames[index - 1];
        let mut min_x = width;
        let mut min_y = height;
        let mut max_x = 0;
        let mut max_y = 0;
        let mut changed = false;
        for y in 0..height as usize {
            for x in 0..width as usize {
                let offset = (y * width as usize + x) * 4;
                if frame[offset..offset + 4] != previous[offset..offset + 4] {
                    changed = true;
                    min_x = min_x.min(x as u32);
                    min_y = min_y.min(y as u32);
                    max_x = max_x.max(x as u32);
                    max_y = max_y.max(y as u32);
                }
            }
        }
        if !changed {
            rects.push(Rect {
                x: 0,
                y: 0,
                width: 2.min(width),
                height: 2.min(height),
            });
            continue;
        }
        let x = min_x & !1;
        let y = min_y & !1;
        rects.push(Rect {
            x,
            y,
            width: max_x - x + 1,
            height: max_y - y + 1,
        });
    }
    rects
}

fn dirty_ratio(rects: &[Rect], width: u32, height: u32) -> f64 {
    let area: u64 = rects
        .iter()
        .map(|rect| rect.width as u64 * rect.height as u64)
        .sum();
    let total = width as u64 * height as u64 * rects.len().max(1) as u64;
    if total == 0 {
        return 0.0;
    }
    ((area as f64 / total as f64) * 100.0).round()
}

fn encode_animated_webp(
    frames: &[Vec<u8>],
    rects: &[Rect],
    width: u32,
    height: u32,
) -> Result<Vec<u8>> {
    if frames.len() != FRAMES || rects.len() != FRAMES {
        bail!("动画帧数不符");
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(b"WEBP");

    let mut vp8x = Vec::with_capacity(10);
    vp8x.push(0x12);
    vp8x.extend_from_slice(&[0, 0, 0]);
    write_u24(&mut vp8x, width.saturating_sub(1));
    write_u24(&mut vp8x, height.saturating_sub(1));
    write_chunk(&mut out, b"VP8X", &vp8x);

    let anim = [0u8; 6];
    write_chunk(&mut out, b"ANIM", &anim);

    for (index, (frame, rect)) in frames.iter().zip(rects).enumerate() {
        let payload = encode_vp8l(frame, *rect, width)?;
        let duration = if index + 1 == FRAMES {
            LAST_FRAME_MS
        } else {
            FRAME_MS
        };
        let mut data = Vec::with_capacity(16 + payload.len() + 8);
        write_u24(&mut data, rect.x / 2);
        write_u24(&mut data, rect.y / 2);
        write_u24(&mut data, rect.width.saturating_sub(1));
        write_u24(&mut data, rect.height.saturating_sub(1));
        write_u24(&mut data, duration as u32);
        data.push(0x02);
        data.extend_from_slice(b"VP8L");
        data.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        data.extend_from_slice(&payload);
        if payload.len() % 2 == 1 {
            data.push(0);
        }
        write_chunk(&mut out, b"ANMF", &data);
    }

    let riff_size = (out.len() - 8) as u32;
    out[4..8].copy_from_slice(&riff_size.to_le_bytes());
    Ok(out)
}

fn encode_vp8l(frame: &[u8], rect: Rect, canvas_width: u32) -> Result<Vec<u8>> {
    let mut rgba = Vec::with_capacity(rect.width as usize * rect.height as usize * 4);
    for y in rect.y..rect.y + rect.height {
        let start = (y as usize * canvas_width as usize + rect.x as usize) * 4;
        let end = start + rect.width as usize * 4;
        rgba.extend_from_slice(&frame[start..end]);
    }
    let mut container = Vec::new();
    WebPEncoder::new(&mut container)
        .encode(&rgba, rect.width, rect.height, ColorType::Rgba8)
        .context("编码 WebP 帧")?;
    if container.get(12..16) != Some(b"VP8L") {
        bail!("WebP 帧不是 VP8L");
    }
    let length =
        u32::from_le_bytes(container[16..20].try_into().context("WebP 帧长度字段")?) as usize;
    let end = 20 + length;
    if end > container.len() {
        bail!("WebP 帧长度越界");
    }
    Ok(container[20..end].to_vec())
}

fn write_u24(out: &mut Vec<u8>, value: u32) {
    out.push((value & 0xFF) as u8);
    out.push(((value >> 8) & 0xFF) as u8);
    out.push(((value >> 16) & 0xFF) as u8);
}

fn write_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(kind);
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    if data.len() % 2 == 1 {
        out.push(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source() -> Image {
        let mut pixels = vec![0u8; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let offset = (y * 16 + x) * 4;
                pixels[offset] = (x * 12) as u8;
                pixels[offset + 1] = (y * 12) as u8;
                pixels[offset + 2] = 80;
                pixels[offset + 3] = 255;
            }
        }
        Image {
            width: 16,
            height: 16,
            pixels,
        }
    }

    #[test]
    fn animated_webp_has_target_chunks_and_timing() {
        let source = source();
        let frames = render_frames(&source, "diagonal").expect("帧");
        let rects = frame_rects(&frames, source.width, source.height);
        let output =
            encode_animated_webp(&frames, &rects, source.width, source.height).expect("编码");
        assert_eq!(&output[..4], b"RIFF");
        assert_eq!(&output[8..12], b"WEBP");
        assert!(output.windows(4).any(|window| window == b"VP8X"));
        assert!(output.windows(4).any(|window| window == b"ANIM"));
        assert_eq!(
            output
                .windows(4)
                .filter(|window| *window == b"ANMF")
                .count(),
            FRAMES
        );
        let frames = decode_webp(&output).expect("解码");
        assert_eq!((frames.width, frames.height), (16, 16));
        assert_eq!(rects[0].width, 16);
        assert_eq!(rects[0].height, 16);
        for rect in &rects[1..] {
            assert_eq!(rect.x % 2, 0);
            assert_eq!(rect.y % 2, 0);
        }
    }

    #[test]
    fn last_frame_duration_is_long() {
        let source = source();
        let frames = render_frames(&source, "breathe").expect("帧");
        let rects = frame_rects(&frames, source.width, source.height);
        let output =
            encode_animated_webp(&frames, &rects, source.width, source.height).expect("编码");
        let mut offset = 12;
        let mut durations = Vec::new();
        while offset + 8 <= output.len() {
            let kind = &output[offset..offset + 4];
            let length =
                u32::from_le_bytes(output[offset + 4..offset + 8].try_into().unwrap()) as usize;
            if kind == b"ANMF" {
                let data = &output[offset + 8..offset + 8 + length];
                durations.push(data[12] as u32 | (data[13] as u32) << 8 | (data[14] as u32) << 16);
            }
            offset += 8 + length + (length & 1);
        }
        assert_eq!(durations.len(), FRAMES);
        assert_eq!(durations[..9], [80; 9]);
        assert_eq!(durations[9], 1_500);
    }

    #[test]
    fn effects_keep_transparent_pixels_clear() {
        let mut source = source();
        for pixel in source.pixels.as_chunks_mut::<4>().0 {
            if pixel[0] < 20 {
                pixel[3] = 0;
            }
        }
        for effect in ["diagonal", "vertical", "bounce", "spot", "glow", "breathe"] {
            let frames = render_frames(&source, effect).expect("帧");
            for frame in &frames {
                for (before, after) in source
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(frame.as_chunks::<4>().0)
                {
                    if before[3] == 0 {
                        assert_eq!(before, after, "{effect}");
                    }
                }
            }
        }
    }

    #[test]
    fn dirty_ratio_counts_rectangles() {
        let rects = [
            Rect {
                x: 0,
                y: 0,
                width: 4,
                height: 4,
            },
            Rect {
                x: 0,
                y: 0,
                width: 2,
                height: 2,
            },
        ];
        assert_eq!(dirty_ratio(&rects, 4, 4), 63.0);
    }

    #[test]
    fn unknown_effect_is_an_error() {
        assert!(render_frames(&source(), "bad").is_err());
    }

    #[test]
    fn scaling_uses_longest_side() {
        assert_eq!(scaled_size(800, 400, 512), (512, 256));
        assert_eq!(scaled_size(400, 800, 512), (256, 512));
        assert_eq!(scaled_size(64, 64, 512), (64, 64));
    }

    #[test]
    fn png_and_webp_decode_to_rgba() {
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 2, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().expect("PNG 头");
            writer
                .write_image_data(&[1, 2, 3, 4, 5, 6, 7, 8])
                .expect("PNG 数据");
        }
        let image = decode_png(&png).expect("解码 PNG");
        assert_eq!((image.width, image.height), (2, 1));
        assert_eq!(image.pixels, [1, 2, 3, 4, 5, 6, 7, 8]);

        let mut webp = Vec::new();
        WebPEncoder::new(&mut webp)
            .encode(&image.pixels, 2, 1, ColorType::Rgba8)
            .expect("编码 WebP");
        let decoded = decode_webp(&webp).expect("解码 WebP");
        assert_eq!((decoded.width, decoded.height), (2, 1));
        assert_eq!(decoded.pixels, image.pixels);
    }
}
