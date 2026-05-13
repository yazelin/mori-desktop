//! X11 XShape extension — 給 transparent decorationless window 做 OS-level
//! 圓 / 圓角矩形 bounding region。
//!
//! 為什麼不用 CSS border-radius:WebKit2GTK on X11 對 transparent window 的
//! ARGB visual 處理在 alpha ≠ 0 / ≠ 1 的 pixel 上不穩(渲染成 opaque 方塊)。
//! border-radius 邊緣 AA 會產生這種 half-alpha pixel → 圓角破成方框。
//!
//! XShape 是 X server 層級的 1-bit alpha clip:每個 pixel 屬於 in 或 out,
//! 沒中間態。OS / mutter 直接根據 mask 決定哪些 pixel 要 paint、哪些 pass-
//! through。Compositor 不參與,沒 AA、沒半透明,完全避開 WebKit 問題。
//!
//! 實作:用 [`shape_rectangles`] 把目標形狀拆成水平 scanline rectangles 逼近。
//! 圓 160×160 → 160 條 1px-tall rectangles,每條 width = 2 × sqrt(r² - dy²)。
//! X server 端組合成 region,效能 OK(一次性 setup 不是每幀)。

use anyhow::{Context as _, Result};
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::Rectangle;
use x11rb::rust_connection::RustConnection;

/// 給 X11 window 套圓形 bounding clip。`(w, h)` 是視窗尺寸(physical pixel)。
/// 圓心 = (w/2, h/2),半徑 = min(w, h) / 2。
pub fn apply_circle_clip(xid: u32, w: u32, h: u32) -> Result<()> {
    let (conn, _) = RustConnection::connect(None).context("X11 connect")?;
    let radius = w.min(h) as f64 / 2.0;
    let cx = w as f64 / 2.0;
    let cy = h as f64 / 2.0;
    let rects = circle_scanlines(cx, cy, radius, h);
    conn.shape_rectangles(
        SO::SET,    // SET = replace existing bounding region
        SK::BOUNDING, // BOUNDING = which part of window OS renders / accepts input
        x11rb::protocol::xproto::ClipOrdering::UNSORTED,
        xid,
        0, 0,       // offset
        &rects,
    )
    .context("XShape rectangles request")?
    .check()
    .context("XShape rectangles check")?;
    conn.flush().context("X11 flush")?;
    tracing::info!(xid, w, h, n_rects = rects.len(), "applied XShape circle clip");
    Ok(())
}

/// 清除 X11 window 的 bounding clip — 把 region 設成整個視窗矩形,等同
/// 「沒套 XShape」的預設狀態。給 user 切回 square 模式時用。
pub fn clear_clip(xid: u32, w: u32, h: u32) -> Result<()> {
    let (conn, _) = RustConnection::connect(None).context("X11 connect")?;
    let rect = Rectangle {
        x: 0,
        y: 0,
        width: w as u16,
        height: h as u16,
    };
    conn.shape_rectangles(
        SO::SET,
        SK::BOUNDING,
        x11rb::protocol::xproto::ClipOrdering::UNSORTED,
        xid,
        0,
        0,
        &[rect],
    )
    .context("XShape clear request")?
    .check()
    .context("XShape clear check")?;
    conn.flush().context("X11 flush")?;
    tracing::info!(xid, w, h, "cleared XShape bounding clip (square)");
    Ok(())
}

/// 給 X11 window 套圓角矩形 bounding clip。`(w, h)` 視窗尺寸,`radius` 角弧。
/// 中間矩形 + 4 個角圓弧區段。半徑超過 min(w,h)/2 自動 clamp。
pub fn apply_rounded_clip(xid: u32, w: u32, h: u32, radius: u32) -> Result<()> {
    let (conn, _) = RustConnection::connect(None).context("X11 connect")?;
    let r = radius.min(w / 2).min(h / 2);
    let rects = rounded_rect_scanlines(w, h, r);
    conn.shape_rectangles(
        SO::SET,
        SK::BOUNDING,
        x11rb::protocol::xproto::ClipOrdering::UNSORTED,
        xid,
        0, 0,
        &rects,
    )
    .context("XShape rectangles request")?
    .check()
    .context("XShape rectangles check")?;
    conn.flush().context("X11 flush")?;
    tracing::info!(xid, w, h, radius = r, n_rects = rects.len(), "applied XShape rounded clip");
    Ok(())
}

/// 圓的 scanline rectangles:for each y row, x = cx ± sqrt(r² - (y - cy)²)。
fn circle_scanlines(cx: f64, cy: f64, r: f64, h: u32) -> Vec<Rectangle> {
    let mut rects = Vec::with_capacity(h as usize);
    for y in 0..h as i32 {
        let dy = y as f64 + 0.5 - cy; // center of pixel row
        let dy2 = dy * dy;
        let r2 = r * r;
        if dy2 >= r2 {
            continue; // 完全在圓外
        }
        let dx = (r2 - dy2).sqrt();
        let x_left = (cx - dx).round() as i16;
        let x_right = (cx + dx).round() as i16;
        let width = (x_right - x_left).max(0) as u16;
        if width == 0 {
            continue;
        }
        rects.push(Rectangle {
            x: x_left,
            y: y as i16,
            width,
            height: 1,
        });
    }
    rects
}

/// 圓角矩形的 scanline rectangles:
/// - 中段(y in [r, h-r])整列填滿 [0, w]
/// - 上下圓角區段每 row 算 x 範圍
fn rounded_rect_scanlines(w: u32, h: u32, r: u32) -> Vec<Rectangle> {
    let mut rects = Vec::new();
    let r = r as f64;
    let w_f = w as f64;
    for y in 0..h as i32 {
        let yf = y as f64 + 0.5;
        let cy = if yf < r {
            // 上圓角 — 圓心 y = r
            r
        } else if yf >= h as f64 - r {
            // 下圓角 — 圓心 y = h - r
            h as f64 - r
        } else {
            // 中段,整列
            rects.push(Rectangle {
                x: 0,
                y: y as i16,
                width: w as u16,
                height: 1,
            });
            continue;
        };
        let dy = yf - cy;
        let dy2 = dy * dy;
        let r2 = r * r;
        if dy2 >= r2 {
            continue;
        }
        let dx = (r2 - dy2).sqrt();
        // 左圓角 x 範圍:cx - dx 到 cx;右圓角:w - cx 到 w - cx + dx。
        // 但 cx 對左右一樣,因為兩邊圓心是 (r, ...) 跟 (w-r, ...);這裡簡化
        // 為左右對稱:整 row 從 (r - dx) 到 (w - r + dx)。
        let x_left = (r - dx).round() as i16;
        let x_right = (w_f - r + dx).round() as i16;
        let width = (x_right - x_left).max(0) as u16;
        if width == 0 {
            continue;
        }
        rects.push(Rectangle {
            x: x_left,
            y: y as i16,
            width,
            height: 1,
        });
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn circle_scanlines_count() {
        let rects = circle_scanlines(80.0, 80.0, 80.0, 160);
        // 整圓垂直跨 160 行,中間每行都有 rect,邊緣 row 可能因為 dx≈0 被
        // 過濾。預期 ~160 條,不能少於 100。
        assert!(rects.len() > 100, "got {} rects", rects.len());
    }

    #[test]
    fn rounded_rect_includes_middle_full_rows() {
        // 100x100 with radius 20:中段 y∈[20, 80] 共 60 行整 row 100 width
        let rects = rounded_rect_scanlines(100, 100, 20);
        let full_rows = rects.iter().filter(|r| r.width == 100).count();
        assert!(full_rows >= 50, "expected middle full rows, got {full_rows}");
    }
}
