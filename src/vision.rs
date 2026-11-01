//! Seeing the game: binarised text/icon patterns + colour image templates.
//!
//! **Text patterns** (FindText-style, own format): a crop is turned to gray
//! with FindText's formula `(R*38 + G*75 + B*15) >> 7`, then to bits with a
//! threshold (bit = 1 where the pixel is brighter than the threshold). A match
//! allows `err1` of the pattern's 1-pixels and `err0` of its 0-pixels to
//! differ, which survives anti-aliasing and small background changes.
//! Matching packs rows into u64 words and counts mismatches with popcount.
//!
//! **Image templates**: colour crops compared per pixel with a channel
//! tolerance (like AHK ImageSearch `*n`), for icons where colour matters.
//!
//! Every pattern stores the region it is searched in (client coords), so
//! a lookup scans a small box, not the whole 800x599 client.

use crate::capture::Frame;
use crate::geom::{Point, Rect};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// FindText gray: `(R*38 + G*75 + B*15) >> 7`.
#[inline]
pub fn gray_px(r: u8, g: u8, b: u8) -> u8 {
    ((r as u32 * 38 + g as u32 * 75 + b as u32 * 15) >> 7) as u8
}

/// Gray values of `rect` (clipped), row-major.
pub fn gray(frame: &Frame, rect: Rect) -> (Rect, Vec<u8>) {
    let r = rect
        .intersect(&Rect::new(0, 0, frame.width as i32, frame.height as i32))
        .unwrap_or_default();
    let mut out = Vec::with_capacity((r.w * r.h).max(0) as usize);
    for y in r.y..r.bottom() {
        let row = (y as u32 * frame.width) as usize * 4;
        for x in r.x..r.right() {
            let i = row + x as usize * 4;
            out.push(gray_px(frame.bgra[i + 2], frame.bgra[i + 1], frame.bgra[i]));
        }
    }
    (r, out)
}

/// Otsu threshold: the split that best separates text from background.
pub fn otsu(values: &[u8]) -> u8 {
    let mut hist = [0u64; 256];
    for &v in values {
        hist[v as usize] += 1;
    }
    let total = values.len() as f64;
    let sum_all: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum();
    let (mut w_b, mut sum_b, mut best, mut best_t) = (0f64, 0f64, -1f64, 128u8);
    for t in 0..256 {
        w_b += hist[t] as f64;
        if w_b == 0.0 {
            continue;
        }
        let w_f = total - w_b;
        if w_f == 0.0 {
            break;
        }
        sum_b += t as f64 * hist[t] as f64;
        let (m_b, m_f) = (sum_b / w_b, (sum_all - sum_b) / w_f);
        let between = w_b * w_f * (m_b - m_f) * (m_b - m_f);
        if between > best {
            best = between;
            best_t = t as u8;
        }
    }
    best_t
}

/// Rows of bits, packed into u64 words (bit i of word k = column 64k + i).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BitImage {
    pub w: usize,
    pub h: usize,
    words: usize,
    data: Vec<u64>,
}

impl BitImage {
    pub fn from_fn(w: usize, h: usize, f: impl Fn(usize, usize) -> bool) -> Self {
        let words = w.div_ceil(64).max(1);
        let mut data = vec![0u64; words * h];
        for y in 0..h {
            for x in 0..w {
                if f(x, y) {
                    data[y * words + x / 64] |= 1 << (x % 64);
                }
            }
        }
        Self { w, h, words, data }
    }
    pub fn from_gray(w: usize, h: usize, g: &[u8], threshold: u8) -> Self {
        Self::from_fn(w, h, |x, y| g[y * w + x] > threshold)
    }
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.data[y * self.words + x / 64] >> (x % 64) & 1 == 1
    }
    pub fn ones(&self) -> usize {
        self.data.iter().map(|w| w.count_ones() as usize).sum()
    }
    /// `len` (<= 64) bits of row `y` starting at column `x`.
    #[inline]
    fn bits(&self, y: usize, x: usize, len: usize) -> u64 {
        let row = &self.data[y * self.words..(y + 1) * self.words];
        let (k, s) = (x / 64, x % 64);
        let mut v = row[k] >> s;
        if s != 0 && k + 1 < row.len() {
            v |= row[k + 1] << (64 - s);
        }
        if len < 64 {
            v &= (1u64 << len) - 1;
        }
        v
    }
    /// '1'/'0' rows joined by '\n' (pattern files).
    pub fn to_text(&self) -> String {
        (0..self.h)
            .map(|y| {
                (0..self.w)
                    .map(|x| if self.get(x, y) { '1' } else { '0' })
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    pub fn from_text(s: &str) -> Result<Self> {
        let rows: Vec<&str> = s.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let h = rows.len();
        let w = rows.first().map(|r| r.len()).unwrap_or(0);
        if h == 0 || w == 0 || rows.iter().any(|r| r.len() != w) {
            bail!("pattern bits must be equal-length rows of 0/1");
        }
        Ok(Self::from_fn(w, h, |x, y| rows[y].as_bytes()[x] == b'1'))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hit {
    /// Top-left of the match, client coords.
    pub at: Point,
    pub w: i32,
    pub h: i32,
    /// 1.0 = identical, lower = more pixels differ.
    pub score: f32,
}

impl Hit {
    pub fn center(&self) -> Point {
        Point::new(self.at.x + self.w / 2, self.at.y + self.h / 2)
    }
}

/// Search `pat` inside `img`. Returns the best position within tolerance.
pub fn find_bits(
    img: &BitImage,
    pat: &BitImage,
    err1: f32,
    err0: f32,
) -> Option<(usize, usize, usize)> {
    if pat.w > img.w || pat.h > img.h || pat.w == 0 {
        return None;
    }
    let ones = pat.ones();
    let zeros = pat.w * pat.h - ones;
    let allow1 = (ones as f32 * err1) as usize;
    let allow0 = (zeros as f32 * err0) as usize;
    let chunks: Vec<(usize, usize)> = (0..pat.w)
        .step_by(64)
        .map(|c| (c, (pat.w - c).min(64)))
        .collect();
    let mut best: Option<(usize, usize, usize)> = None;
    for oy in 0..=img.h - pat.h {
        'pos: for ox in 0..=img.w - pat.w {
            let (mut miss1, mut miss0) = (0usize, 0usize);
            for py in 0..pat.h {
                for &(c, len) in &chunks {
                    let p = pat.bits(py, c, len);
                    let i = img.bits(oy + py, ox + c, len);
                    let mask = if len == 64 {
                        u64::MAX
                    } else {
                        (1u64 << len) - 1
                    };
                    miss1 += (p & !i).count_ones() as usize;
                    miss0 += (!p & i & mask).count_ones() as usize;
                }
                if miss1 > allow1 || miss0 > allow0 {
                    continue 'pos;
                }
            }
            let total = miss1 + miss0;
            if best.is_none_or(|b| total < b.2) {
                best = Some((ox, oy, total));
                if total == 0 {
                    return best;
                }
            }
        }
    }
    best
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Binarised (text, outlines).
    Text,
    /// Colour template (`<name>.png` beside the json).
    Image,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pattern {
    pub name: String,
    pub kind: Kind,
    /// Where to look, client coords (crop + margin by default).
    pub search: Rect,
    /// Where it was captured, client coords (for reference / click offsets).
    pub captured_at: Rect,
    #[serde(default)]
    pub threshold: u8,
    #[serde(default = "d_err")]
    pub err1: f32,
    #[serde(default = "d_err")]
    pub err0: f32,
    /// Image kind: per-channel tolerance (0-255) and allowed bad-pixel fraction.
    #[serde(default = "d_tol")]
    pub tolerance: u8,
    #[serde(default = "d_bad")]
    pub max_bad: f32,
    /// Text kind: '0'/'1' rows.
    #[serde(default)]
    pub bits: String,
    #[serde(skip)]
    compiled: Option<BitImage>,
    #[serde(skip)]
    template: Option<Frame>,
}

fn d_err() -> f32 {
    0.08
}
fn d_tol() -> u8 {
    24
}
fn d_bad() -> f32 {
    0.05
}

impl Pattern {
    /// Text pattern from a crop of `frame`; threshold `None` = Otsu.
    pub fn text_from(
        frame: &Frame,
        name: &str,
        crop: Rect,
        margin: i32,
        threshold: Option<u8>,
    ) -> Result<Self> {
        let (r, g) = gray(frame, crop);
        if r.w < 2 || r.h < 2 {
            bail!("crop is outside the capture");
        }
        let t = threshold.unwrap_or_else(|| otsu(&g));
        let bits = BitImage::from_gray(r.w as usize, r.h as usize, &g, t);
        let ones = bits.ones();
        if ones == 0 || ones == bits.w * bits.h {
            bail!("threshold {t} makes the crop all one colour - pick another --threshold");
        }
        Ok(Self {
            name: name.into(),
            kind: Kind::Text,
            search: grow(r, margin, frame),
            captured_at: r,
            threshold: t,
            err1: d_err(),
            err0: d_err(),
            tolerance: d_tol(),
            max_bad: d_bad(),
            bits: bits.to_text(),
            compiled: Some(bits),
            template: None,
        })
    }

    pub fn image_from(frame: &Frame, name: &str, crop: Rect, margin: i32) -> Result<Self> {
        let r = crop
            .intersect(&Rect::new(0, 0, frame.width as i32, frame.height as i32))
            .context("crop outside")?;
        Ok(Self {
            name: name.into(),
            kind: Kind::Image,
            search: grow(r, margin, frame),
            captured_at: r,
            threshold: 0,
            err1: d_err(),
            err0: d_err(),
            tolerance: d_tol(),
            max_bad: d_bad(),
            bits: String::new(),
            compiled: None,
            template: Some(frame.crop(r.x as u32, r.y as u32, r.w as u32, r.h as u32)),
        })
    }

    pub fn size(&self) -> (i32, i32) {
        (self.captured_at.w, self.captured_at.h)
    }

    /// Search `region` (default: the pattern's own search box) in `frame`.
    pub fn find_in(&self, frame: &Frame, region: Option<Rect>) -> Option<Hit> {
        let area = region.unwrap_or(self.search);
        let area = area.intersect(&Rect::new(0, 0, frame.width as i32, frame.height as i32))?;
        match self.kind {
            Kind::Text => {
                let pat = self.compiled.as_ref()?;
                let (a, g) = gray(frame, area);
                let img = BitImage::from_gray(a.w as usize, a.h as usize, &g, self.threshold);
                let (x, y, miss) = find_bits(&img, pat, self.err1, self.err0)?;
                let n = (pat.w * pat.h) as f32;
                Some(Hit {
                    at: Point::new(a.x + x as i32, a.y + y as i32),
                    w: pat.w as i32,
                    h: pat.h as i32,
                    score: 1.0 - miss as f32 / n,
                })
            }
            Kind::Image => {
                let t = self.template.as_ref()?;
                find_image(frame, area, t, self.tolerance, self.max_bad)
            }
        }
    }

    pub fn compile(&mut self) -> Result<()> {
        if self.kind == Kind::Text {
            self.compiled = Some(BitImage::from_text(&self.bits)?);
        }
        Ok(())
    }

    pub fn bit_image(&self) -> Option<&BitImage> {
        self.compiled.as_ref()
    }
}

fn grow(r: Rect, m: i32, f: &Frame) -> Rect {
    Rect::new(r.x - m, r.y - m, r.w + 2 * m, r.h + 2 * m)
        .intersect(&Rect::new(0, 0, f.width as i32, f.height as i32))
        .unwrap_or(r)
}

/// Colour template search: a pixel matches when every channel is within
/// `tol`; a position matches when at most `max_bad` of pixels miss.
pub fn find_image(frame: &Frame, area: Rect, t: &Frame, tol: u8, max_bad: f32) -> Option<Hit> {
    let (tw, th) = (t.width as i32, t.height as i32);
    if tw > area.w || th > area.h || tw == 0 {
        return None;
    }
    let allow = ((tw * th) as f32 * max_bad) as i32;
    let tol = tol as i16;
    let mut best: Option<(i32, i32, i32)> = None;
    for oy in area.y..=area.bottom() - th {
        'pos: for ox in area.x..=area.right() - tw {
            let mut bad = 0;
            for ty in 0..th {
                let fr = ((oy + ty) as u32 * frame.width) as usize * 4;
                let tr = (ty as u32 * t.width) as usize * 4;
                for tx in 0..tw {
                    let fi = fr + (ox + tx) as usize * 4;
                    let ti = tr + tx as usize * 4;
                    let d =
                        |k: usize| (frame.bgra[fi + k] as i16 - t.bgra[ti + k] as i16).abs() > tol;
                    if d(0) || d(1) || d(2) {
                        bad += 1;
                        if bad > allow {
                            continue 'pos;
                        }
                    }
                }
            }
            if best.is_none_or(|b| bad < b.2) {
                best = Some((ox, oy, bad));
                if bad == 0 {
                    break;
                }
            }
        }
        if best.is_some_and(|b| b.2 == 0) {
            break;
        }
    }
    best.map(|(x, y, bad)| Hit {
        at: Point::new(x, y),
        w: tw,
        h: th,
        score: 1.0 - bad as f32 / (tw * th) as f32,
    })
}

/// Named patterns loaded from a folder (`<name>.json`, + `<name>.png` for images).
#[derive(Default)]
pub struct Library {
    pub dir: PathBuf,
    pats: BTreeMap<String, Pattern>,
}

impl Library {
    pub fn default_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("patterns")))
            .unwrap_or_else(|| PathBuf::from("patterns"))
    }

    pub fn load(dir: &Path) -> Result<Self> {
        let mut lib = Library {
            dir: dir.to_path_buf(),
            pats: BTreeMap::new(),
        };
        if !dir.exists() {
            return Ok(lib);
        }
        for e in std::fs::read_dir(dir)? {
            let p = e?.path();
            if p.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let mut pat: Pattern = serde_json::from_str(&std::fs::read_to_string(&p)?)
                .with_context(|| format!("reading {}", p.display()))?;
            pat.compile()?;
            if pat.kind == Kind::Image {
                pat.template = Some(Frame::load_png(&p.with_extension("png"))?);
            }
            lib.pats.insert(key(&pat.name), pat);
        }
        Ok(lib)
    }

    pub fn save(&mut self, pat: Pattern) -> Result<PathBuf> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(format!("{}.json", key(&pat.name)));
        std::fs::write(&path, serde_json::to_string_pretty(&pat)?)?;
        if let Some(t) = &pat.template {
            t.save_png(&path.with_extension("png"))?;
        }
        if let Some(b) = &pat.compiled {
            preview(b).save_png(&self.dir.join(format!("{}.preview.png", key(&pat.name))))?;
        }
        self.pats.insert(key(&pat.name), pat);
        Ok(path)
    }

    pub fn get(&self, name: &str) -> Option<&Pattern> {
        self.pats.get(&key(name))
    }
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.pats.values().map(|p| p.name.as_str())
    }
    pub fn len(&self) -> usize {
        self.pats.len()
    }
    pub fn is_empty(&self) -> bool {
        self.pats.is_empty()
    }

    /// First of `names` found on `frame`.
    pub fn find_any(&self, frame: &Frame, names: &[&str]) -> Option<(String, Hit)> {
        names.iter().find_map(|n| {
            self.get(n)
                .and_then(|p| p.find_in(frame, None))
                .map(|h| (n.to_string(), h))
        })
    }
}

/// Lower-case letters/digits/_ only: "Vote Start!" -> "vote_start".
pub fn key(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            s.push(c.to_ascii_lowercase());
        } else if !s.ends_with('_') && !s.is_empty() {
            s.push('_');
        }
    }
    s.trim_end_matches('_').to_string()
}

/// Black/white picture of a bit pattern, scaled 4x so it's easy to check.
pub fn preview(b: &BitImage) -> Frame {
    let s = 4;
    let (w, h) = (b.w * s, b.h * s);
    let mut v = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let c = if b.get(x / s, y / s) { 255 } else { 0 };
            v.extend_from_slice(&[c, c, c, 255]);
        }
    }
    Frame::new(w as u32, h as u32, v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dark frame with a bright "glyph" (a plus sign + bar) at (ox, oy).
    fn scene(ox: i32, oy: i32) -> Frame {
        let mut f = Frame::solid(200, 120, 30, 30, 30);
        let mut put = |x: i32, y: i32| {
            let i = ((y as u32 * 200 + x as u32) * 4) as usize;
            f.bgra[i..i + 3].copy_from_slice(&[240, 240, 240]);
        };
        for k in 0..20 {
            put(ox + k, oy + 10); // horizontal
            put(ox + 10, oy + k); // vertical
            put(ox + k, oy + 18); // bottom bar
        }
        f
    }

    #[test]
    fn gray_formula_matches_findtext() {
        assert_eq!(gray_px(255, 255, 255), 255);
        assert_eq!(gray_px(0, 0, 0), 0);
        assert_eq!(gray_px(100, 0, 0), (100 * 38 >> 7) as u8);
    }

    #[test]
    fn otsu_splits_two_levels() {
        let mut v = vec![30u8; 500];
        v.extend(vec![220u8; 200]);
        let t = otsu(&v);
        assert!((30..220).contains(&t), "t={t}");
    }

    #[test]
    fn bits_roundtrip_text() {
        let b = BitImage::from_fn(70, 3, |x, y| (x + y) % 3 == 0);
        let back = BitImage::from_text(&b.to_text()).unwrap();
        assert_eq!(b, back);
    }

    #[test]
    fn text_pattern_found_after_move() {
        let src = scene(40, 30);
        let mut pat =
            Pattern::text_from(&src, "Plus Sign", Rect::new(38, 28, 24, 24), 0, None).unwrap();
        assert_eq!(pat.name, "Plus Sign");
        // Search the whole client, glyph moved.
        let moved = scene(130, 70);
        let hit = pat
            .find_in(&moved, Some(Rect::new(0, 0, 200, 120)))
            .expect("found");
        assert_eq!(hit.at, Point::new(128, 68));
        assert!(hit.score > 0.99);
        // Not present.
        let empty = Frame::solid(200, 120, 30, 30, 30);
        assert!(pat
            .find_in(&empty, Some(Rect::new(0, 0, 200, 120)))
            .is_none());
        // Survives a few flipped pixels.
        let mut noisy = scene(40, 30);
        for k in 0..3 {
            let i = (((31 + k) as u32 * 200 + 41) * 4) as usize;
            noisy.bgra[i..i + 3].copy_from_slice(&[240, 240, 240]);
        }
        pat.err0 = 0.05;
        assert!(pat.find_in(&noisy, None).is_some());
    }

    #[test]
    fn wide_pattern_over_64_columns() {
        let img = BitImage::from_fn(300, 20, |x, y| {
            let h = (x as u64 * 0x9E37_79B9 ^ y as u64 * 0x85EB_CA6B).wrapping_mul(0xC2B2_AE35);
            (h >> 29) & 1 == 1
        });
        let pat = BitImage::from_fn(130, 6, |x, y| img.get(x + 101, y + 9));
        let (x, y, miss) = find_bits(&img, &pat, 0.0, 0.0).unwrap();
        assert_eq!((x, y, miss), (101, 9, 0));
    }

    #[test]
    fn image_template_found() {
        let mut f = Frame::solid(120, 80, 10, 20, 30);
        for y in 50..60 {
            for x in 70..85 {
                let i = ((y * 120 + x) * 4) as usize;
                f.bgra[i..i + 3].copy_from_slice(&[0, (x * 9) as u8, 200]);
            }
        }
        let pat = Pattern::image_from(&f, "icon", Rect::new(70, 50, 15, 10), 5).unwrap();
        let hit = pat.find_in(&f, Some(Rect::new(0, 0, 120, 80))).unwrap();
        assert_eq!(hit.at, Point::new(70, 50));
        assert!(find_image(
            &Frame::solid(120, 80, 10, 20, 30),
            Rect::new(0, 0, 120, 80),
            &f.crop(70, 50, 15, 10),
            24,
            0.05
        )
        .is_none());
    }

    #[test]
    fn library_save_load() {
        let dir = std::env::temp_dir().join(format!("aa-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut lib = Library::load(&dir).unwrap();
        let src = scene(40, 30);
        lib.save(
            Pattern::text_from(&src, "Vote Start!", Rect::new(38, 28, 24, 24), 10, None).unwrap(),
        )
        .unwrap();
        lib.save(Pattern::image_from(&src, "icon", Rect::new(38, 28, 24, 24), 10).unwrap())
            .unwrap();
        let lib2 = Library::load(&dir).unwrap();
        assert_eq!(lib2.len(), 2);
        assert!(lib2.get("vote start").is_some());
        assert!(lib2
            .get("VOTE_START")
            .unwrap()
            .find_in(&src, None)
            .is_some());
        assert!(lib2.get("icon").unwrap().find_in(&src, None).is_some());
        assert!(dir.join("vote_start.preview.png").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn key_normalises() {
        assert_eq!(key("Vote  Start!"), "vote_start");
        assert_eq!(key("Return to Lobby"), "return_to_lobby");
    }
}

#[cfg(test)]
mod bench {
    use super::*;

    /// `cargo test --release -- --ignored bench` : worst case, whole 800x599 client.
    #[test]
    #[ignore]
    fn bench_full_client_search() {
        let mut f = Frame::solid(800, 599, 40, 40, 40);
        for y in 0..599u32 {
            for x in 0..800u32 {
                let h = (x as u64 * 0x9E37_79B9 ^ y as u64 * 0x85EB_CA6B).wrapping_mul(0xC2B2_AE35);
                if (h >> 29) & 3 == 0 {
                    let i = ((y * 800 + x) * 4) as usize;
                    f.bgra[i..i + 3].copy_from_slice(&[230, 230, 230]);
                }
            }
        }
        let pat = Pattern::text_from(&f, "p", Rect::new(600, 500, 110, 24), 0, Some(128)).unwrap();
        let t = std::time::Instant::now();
        let hit = pat.find_in(&f, Some(Rect::new(0, 0, 800, 599))).unwrap();
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        println!("full-client search: {ms:.1} ms, hit {:?}", hit.at);
        assert_eq!(hit.at, Point::new(600, 500));
    }
}
