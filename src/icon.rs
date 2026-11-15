//! The Wortfluss icon, drawn in code (no image files to ship): a microphone with the
//! monogram WF beside it; the arms of the F are the lines of text flowing out of the mic. Rendered smoothly at any size for the tray (all states), the
//! notifications and the desktop entry. Self-contained (only `png` and `anyhow`): the
//! build script also uses it, to put the icon into the Windows .exe files.

/// What the tray icon shows.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Ready,
    Dictating,
    Session,
}

type Rgba = [f32; 4];

const MIC: Rgba = [0.95, 0.95, 0.95, 1.0];
const FLOW: Rgba = [0.38, 0.63, 0.92, 1.0]; // blue text lines
const FLOW_SESSION: Rgba = [1.0, 0.64, 0.28, 1.0]; // orange while a session runs
const RED: Rgba = [0.88, 0.11, 0.14, 1.0];
const BG_TOP: Rgba = [0.21, 0.52, 0.89, 1.0];
const BG_BOTTOM: Rgba = [0.11, 0.44, 0.85, 1.0];

/// Signed distance to a rounded box (centre cx/cy, half sizes hx/hy, corner radius r),
/// in the 64x64 design grid.
fn rbox(x: f32, y: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (x - cx).abs() - (hx - r);
    let qy = (y - cy).abs() - (hy - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    outside + qx.max(qy).min(0.0) - r
}

/// Distance to a line from (ax, ay) to (bx, by) with round ends, `w` thick.
fn line(x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32, w: f32) -> f32 {
    let (px, py, dx, dy) = (x - ax, y - ay, bx - ax, by - ay);
    let t = ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    ((px - dx * t).powi(2) + (py - dy * t).powi(2)).sqrt() - w / 2.0
}

fn circle(x: f32, y: f32, cx: f32, cy: f32, r: f32) -> f32 {
    ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() - r
}

/// Colour at a point of the design grid (64 units), on transparent ground.
fn shade(x: f32, y: f32, state: State, app: bool) -> Rgba {
    let mut c: Rgba = [0.0; 4];
    let mut paint = |d: f32, col: Rgba| {
        if d <= 0.0 {
            c = col;
        }
    };
    if app {
        let t = y / 64.0;
        let bg = [
            BG_TOP[0] + (BG_BOTTOM[0] - BG_TOP[0]) * t,
            BG_TOP[1] + (BG_BOTTOM[1] - BG_TOP[1]) * t,
            BG_TOP[2] + (BG_BOTTOM[2] - BG_TOP[2]) * t,
            1.0,
        ];
        paint(rbox(x, y, 32.0, 32.0, 31.0, 31.0, 12.0), bg);
    }
    // In the app icon the drawing is shrunk to 72 % and centred on the blue tile.
    let (x, y) = if app { ((x - 9.5) / 0.72, (y - 8.5) / 0.72) } else { (x, y) };

    // microphone: capsule, holder arc, stem, foot
    paint(rbox(x, y, 16.0, 20.0, 8.0, 15.0, 8.0), MIC);
    let ring = circle(x, y, 16.0, 26.0, 13.0).abs() - 2.2;
    if y > 26.0 {
        paint(ring, MIC);
    }
    paint(rbox(x, y, 16.0, 45.0, 2.2, 6.0, 0.5), MIC);
    paint(rbox(x, y, 16.0, 52.0, 9.0, 2.4, 2.4), MIC);

    // monogram: W on top, F below; the F's arms are the text flowing out of the mic
    let flow = if app { MIC } else if state == State::Session { FLOW_SESSION } else { FLOW };
    let w = 5.0;
    let wd = [(33.5, 7.0), (39.5, 25.0), (46.5, 12.5), (53.5, 25.0), (59.5, 7.0)]
        .windows(2)
        .map(|p| line(x, y, p[0].0, p[0].1, p[1].0, p[1].1, w))
        .fold(f32::MAX, f32::min);
    paint(wd, if app { MIC } else { MIC });
    paint(line(x, y, 36.0, 32.5, 36.0, 52.0, w), flow); // stem
    paint(line(x, y, 36.0, 32.5, 60.0, 32.5, w), flow); // top arm (long line of text)
    paint(line(x, y, 36.0, 42.0, 52.0, 42.0, w), flow); // middle arm (shorter line)

    // recording dot
    if !app && state != State::Ready {
        paint(circle(x, y, 51.0, 52.0, 11.5), [0.1, 0.1, 0.1, 1.0]);
        paint(circle(x, y, 51.0, 52.0, 9.0), RED);
    }
    c
}

/// RGBA pixels, `size` x `size`, smoothed by 4x4 supersampling.
pub fn rgba(size: u32, state: State, app: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    let scale = 64.0 / size as f32;
    const N: u32 = 4;
    for py in 0..size {
        for px in 0..size {
            let mut acc = [0.0f32; 4];
            for sy in 0..N {
                for sx in 0..N {
                    let x = (px as f32 + (sx as f32 + 0.5) / N as f32) * scale;
                    let y = (py as f32 + (sy as f32 + 0.5) / N as f32) * scale;
                    let c = shade(x, y, state, app);
                    // premultiplied accumulation
                    for i in 0..3 {
                        acc[i] += c[i] * c[3];
                    }
                    acc[3] += c[3];
                }
            }
            let n = (N * N) as f32;
            let a = acc[3] / n;
            for i in 0..3 {
                let v = if a > 0.0 { acc[i] / n / a } else { 0.0 };
                out.push((v * 255.0).round() as u8);
            }
            out.push((a * 255.0).round() as u8);
        }
    }
    out
}

/// ARGB32 in network byte order, as the StatusNotifierItem tray protocol wants it.
pub fn argb(size: u32, state: State) -> Vec<u8> {
    rgba(size, state, false)
        .chunks_exact(4)
        .flat_map(|p| [p[3], p[0], p[1], p[2]])
        .collect()
}

pub fn write_png(path: &std::path::Path, size: u32, state: State, app: bool) -> anyhow::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), size, size);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()?.write_image_data(&rgba(size, state, app))?;
    Ok(())
}

/// A Windows .ico with the app icon at 16-256 px (PNG inside, Windows Vista and later).
pub fn write_ico(path: &std::path::Path) -> anyhow::Result<()> {
    let sizes = [16u32, 24, 32, 48, 64, 256];
    let mut images = Vec::new();
    for &s in &sizes {
        let mut png_data = Vec::new();
        let mut enc = png::Encoder::new(&mut png_data, s, s);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.write_header()?.write_image_data(&rgba(s, State::Ready, true))?;
        images.push(png_data);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let side = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[side, side, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    std::fs::write(path, out)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_transparency() {
        let px = rgba(32, State::Ready, false);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(px[3], 0, "corner is transparent");
        let app = rgba(64, State::Ready, true);
        assert_eq!(app[(32 * 64 + 2) * 4 + 3], 255, "app tile is opaque inside");
        assert_eq!(argb(22, State::Dictating).len(), 22 * 22 * 4);
    }
}
