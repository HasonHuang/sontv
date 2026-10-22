//! 台标动效：解码静态图、逐帧生成、编码 APNG。
//!
//! 目标 v0.8.2 用纯 Rust 无损编码（F-012）。本实现借助 `ffmpeg`/`ffprobe`
//! 解码与缩放，再自行生成帧并写 APNG；`ffmpeg` 缺失时返回错误。
//! 动效输出覆盖原文件，并在 `.orig/` 保留首次生成的静态原图。

use std::{f32::consts::TAU, fs, io::Write, path::Path, process::Command};

use anyhow::{bail, Context, Result};
use flate2::{write::ZlibEncoder, Compression};

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

/// 生成 APNG 动效并覆盖原台标。
pub fn animate(
    cache_dir: &str,
    file: &str,
    effect: &str,
    quality: &str,
    max_size: u32,
    frame_ms: u64,
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
    let max_size = max_size.clamp(16, 2048);
    let (width, height) = probe(&path)?;
    let scale = (max_size as f64 / width.max(height) as f64).min(1.0);
    let width = ((width as f64 * scale).round() as u32).max(1);
    let height = ((height as f64 * scale).round() as u32).max(1);
    let source = decode(&path, width, height)?;
    let frame_count = if width.max(height) <= 64 { 12 } else { 20 };
    let delay_ms = frame_ms.clamp(20, 1_000);
    let frames = render_frames(&source, width, height, effect, frame_count)?;
    let dirty_ratio = dirty_ratio(&frames);
    let bytes_before = fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
    let level = (((100 - quality) / 10).min(9)) as u32;
    let output = encode_apng(&frames, width, height, delay_ms, level)?;
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
        total_ms: frames.len() as u64 * delay_ms,
        bytes_before,
        bytes_after: output.len() as u64,
        dirty_ratio,
    })
}

fn probe(path: &Path) -> Result<(u32, u32)> {
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

fn decode(path: &Path, width: u32, height: u32) -> Result<Vec<u8>> {
    let scale = format!("scale={width}:{height}");
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-vf",
            &scale,
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgba",
            "-",
        ])
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
    Ok(output.stdout)
}

fn render_frames(
    source: &[u8],
    width: u32,
    height: u32,
    effect: &str,
    count: usize,
) -> Result<Vec<Vec<u8>>> {
    let w = width as usize;
    let h = height as usize;
    let mut frames = Vec::with_capacity(count);
    for frame in 0..count {
        let t = frame as f32 / count as f32;
        let mut pixels = source.to_vec();
        match effect {
            "diagonal" => sweep(&mut pixels, w, h, t, true),
            "vertical" => sweep(&mut pixels, w, h, t, false),
            "bounce" => bounce(&mut pixels, source, w, h, t),
            "spot" => spot(&mut pixels, w, h, t),
            "glow" => glow(&mut pixels, t),
            "breathe" => breathe(&mut pixels, t),
            other => bail!("未知动效: {other}"),
        }
        frames.push(pixels);
    }
    Ok(frames)
}

fn sweep(pixels: &mut [u8], w: usize, h: usize, t: f32, diagonal: bool) {
    let span = if diagonal { (w + h) as f32 } else { h as f32 };
    for y in 0..h {
        for x in 0..w {
            let pos = if diagonal { (x + y) as f32 } else { y as f32 };
            let phase = (pos / span + t).fract();
            let shine = (phase * TAU).sin().max(0.0).powi(8);
            add_white(&mut pixels[(y * w + x) * 4..][..4], shine * 0.55);
        }
    }
}

fn bounce(pixels: &mut [u8], source: &[u8], w: usize, h: usize, t: f32) {
    let shift = (t * TAU).sin() * h as f32 * 0.06;
    pixels.fill(0);
    for y in 0..h {
        let sy = ((y as f32 - shift).round() as isize).clamp(0, h as isize - 1) as usize;
        pixels[y * w * 4..(y + 1) * w * 4].copy_from_slice(&source[sy * w * 4..(sy + 1) * w * 4]);
    }
}

fn spot(pixels: &mut [u8], w: usize, h: usize, t: f32) {
    let cx = w as f32 * (0.5 + 0.25 * (t * TAU).cos());
    let cy = h as f32 * (0.5 + 0.25 * (t * TAU).sin());
    let sigma = w.max(h) as f32 * 0.18;
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let glow = (-(dx * dx + dy * dy) / (2.0 * sigma * sigma)).exp();
            add_white(&mut pixels[(y * w + x) * 4..][..4], glow * 0.45);
        }
    }
}

fn glow(pixels: &mut [u8], t: f32) {
    let strength = 0.2 + 0.25 * (0.5 + 0.5 * (t * TAU).sin());
    for pixel in pixels.chunks_exact_mut(4) {
        add_white(pixel, strength);
    }
}

fn breathe(pixels: &mut [u8], t: f32) {
    let alpha = 0.65 + 0.35 * (0.5 + 0.5 * (t * TAU).sin());
    for pixel in pixels.chunks_exact_mut(4) {
        pixel[3] = (pixel[3] as f32 * alpha).round().clamp(0.0, 255.0) as u8;
    }
}

fn add_white(pixel: &mut [u8], amount: f32) {
    let amount = amount.clamp(0.0, 1.0);
    for channel in &mut pixel[..3] {
        *channel = (*channel as f32 + (255.0 - *channel as f32) * amount)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
}

fn dirty_ratio(frames: &[Vec<u8>]) -> f64 {
    let Some(first) = frames.first() else {
        return 0.0;
    };
    let total = (first.len() / 4).max(1);
    let changed: usize = frames
        .iter()
        .skip(1)
        .map(|frame| {
            frame
                .chunks_exact(4)
                .zip(first.chunks_exact(4))
                .filter(|(left, right)| left != right)
                .count()
        })
        .sum();
    ((changed as f64 / total as f64) * 1_000.0).round() / 10.0
}

fn encode_apng(
    frames: &[Vec<u8>],
    width: u32,
    height: u32,
    delay_ms: u64,
    level: u32,
) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    let mut actl = Vec::new();
    actl.extend_from_slice(&(frames.len() as u32).to_be_bytes());
    actl.extend_from_slice(&0u32.to_be_bytes());
    chunk(&mut out, b"acTL", &actl);

    let mut sequence = 0u32;
    for (index, frame) in frames.iter().enumerate() {
        let mut raw = Vec::with_capacity(height as usize * (1 + width as usize * 4));
        for row in frame.chunks_exact(width as usize * 4) {
            raw.push(0);
            raw.extend_from_slice(row);
        }
        let compressed = compress(&raw, level)?;
        let mut fctl = Vec::new();
        fctl.extend_from_slice(&sequence.to_be_bytes());
        sequence += 1;
        fctl.extend_from_slice(&width.to_be_bytes());
        fctl.extend_from_slice(&height.to_be_bytes());
        fctl.extend_from_slice(&0u32.to_be_bytes());
        fctl.extend_from_slice(&0u32.to_be_bytes());
        fctl.extend_from_slice(&(delay_ms.min(u16::MAX as u64) as u16).to_be_bytes());
        fctl.extend_from_slice(&1_000u16.to_be_bytes());
        // 每帧前清空画布，避免带透明区域的动效留下拖影。
        fctl.extend_from_slice(&[1, 0]);
        chunk(&mut out, b"fcTL", &fctl);
        if index == 0 {
            chunk(&mut out, b"IDAT", &compressed);
        } else {
            let mut fdat = Vec::with_capacity(compressed.len() + 4);
            fdat.extend_from_slice(&sequence.to_be_bytes());
            sequence += 1;
            fdat.extend_from_slice(&compressed);
            chunk(&mut out, b"fdAT", &fdat);
        }
    }
    chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

fn compress(data: &[u8], level: u32) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::new(level.min(9)));
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                0xEDB8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apng_has_expected_chunks() {
        let frame = vec![255u8; 4 * 4 * 4];
        let output = encode_apng(&[frame.clone(), frame], 4, 4, 80, 3).expect("编码");
        assert_eq!(&output[..8], b"\x89PNG\r\n\x1a\n");
        assert!(output.windows(4).any(|window| window == b"acTL"));
        assert!(output.windows(4).any(|window| window == b"fcTL"));
        assert!(output.windows(4).any(|window| window == b"fdAT"));
        assert_eq!(&output[output.len() - 12..], b"\0\0\0\0IEND\xaeB`\x82");
    }

    #[test]
    fn non_png_inputs_are_animated() {
        if std::process::Command::new("ffmpeg")
            .arg("-version")
            .output()
            .is_err()
            || std::process::Command::new("ffprobe")
                .arg("-version")
                .output()
                .is_err()
        {
            return;
        }
        let root = std::env::temp_dir().join(format!("logo-anim-fmt-{}", uuid::Uuid::new_v4()));
        let cache = root.to_string_lossy().to_string();
        let logos = root.join("logos");
        fs::create_dir_all(&logos).expect("建台标目录");
        for ext in ["bmp", "jpg", "avif"] {
            let path = logos.join(format!("x.{ext}"));
            let status = std::process::Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-f",
                    "lavfi",
                    "-i",
                    "color=c=red:s=8x8:d=1",
                    "-frames:v",
                    "1",
                ])
                .arg(&path)
                .arg("-y")
                .status()
                .expect("执行 ffmpeg");
            if !status.success() {
                continue;
            }
            let report = animate(&cache, &format!("x.{ext}"), "glow", "80", 64, 80, 80)
                .unwrap_or_else(|error| panic!("{ext}: {error:#}"));
            assert!(report.frames > 0);
            let bytes = fs::read(&path).expect("读输出");
            assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{ext}");
        }
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn effects_change_pixels() {
        let source: Vec<u8> = (0..4 * 16 * 16).map(|index| (index % 251) as u8).collect();
        for effect in ["diagonal", "vertical", "bounce", "spot", "glow", "breathe"] {
            let frames = render_frames(&source, 16, 16, effect, 4).expect("帧");
            assert_eq!(frames.len(), 4);
            assert!(frames.iter().any(|frame| frame != &source), "{effect}");
        }
        assert!(render_frames(&source, 16, 16, "bad", 4).is_err());
    }
}
