// SPDX-License-Identifier: MPL-2.0

use crate::{CosmicBg, CosmicBgLayer};

use std::collections::VecDeque;
use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use wmde_bg_config::state::State;
use wmde_bg_config::{Color, Entry, SamplingMethod, ScalingMode, Source};
use cosmic_config::CosmicConfigEntry;
use image::{DynamicImage, ImageDecoder, ImageReader, ImageResult, Limits};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use rand::rng;
use rand::seq::SliceRandom;
use sctk::reexports::calloop::timer::{TimeoutAction, Timer};
use sctk::reexports::calloop::{self, RegistrationToken};
use sctk::reexports::client::QueueHandle;
use tracing::error;
use walkdir::WalkDir;

// TODO filter images by whether they seem to match dark / light mode
// Alternatively only load from light / dark subdirectories given a directory source when this is active

#[derive(Debug)]
pub struct Wallpaper {
    pub entry: Entry,
    pub layers: Vec<CosmicBgLayer>,
    pub image_queue: VecDeque<PathBuf>,
    loop_handle: calloop::LoopHandle<'static, CosmicBg>,
    queue_handle: QueueHandle<CosmicBg>,
    current_source: Option<Source>,
    // Cache of source image, if `current_source` is a `Source::Path`
    current_image: Option<image::DynamicImage>,
    timer_token: Option<RegistrationToken>,
    // Kept alive for the lifetime of the wallpaper; dropping it removes the inotify watches.
    watcher: Option<RecommendedWatcher>,
}

impl Drop for Wallpaper {
    fn drop(&mut self) {
        if let Some(token) = self.timer_token.take() {
            self.loop_handle.remove(token);
        }
    }
}

impl Wallpaper {
    pub fn new(
        entry: Entry,
        queue_handle: QueueHandle<CosmicBg>,
        loop_handle: calloop::LoopHandle<'static, CosmicBg>,
        source_tx: calloop::channel::SyncSender<(String, notify::Event)>,
    ) -> Self {
        let mut wallpaper = Wallpaper {
            entry,
            layers: Vec::new(),
            current_source: None,
            current_image: None,
            image_queue: VecDeque::default(),
            timer_token: None,
            watcher: None,
            loop_handle,
            queue_handle,
        };

        wallpaper.load_images();
        wallpaper.register_timer();
        wallpaper.watch_source(source_tx);
        wallpaper
    }

    pub fn save_state(&self) -> Result<(), cosmic_config::Error> {
        let Some(cur_source) = self.current_source.clone() else {
            return Ok(());
        };
        let state_helper = State::state()?;
        let mut state = State::get_entry(&state_helper).unwrap_or_default();
        for l in &self.layers {
            let name = l.output_info.name.clone().unwrap_or_default();
            if let Some((_, source)) = state
                .wallpapers
                .iter_mut()
                .find(|(output, _)| *output == name)
            {
                *source = cur_source.clone();
            } else {
                state.wallpapers.push((name, cur_source.clone()))
            }
        }
        state.write_entry(&state_helper)
    }

    #[allow(clippy::too_many_lines)]
    pub fn draw(&mut self) {
        let start = Instant::now();
        let mut cur_resized_img: Option<DynamicImage> = None;

        for layer in self.layers.iter_mut().filter(|layer| layer.needs_redraw) {
            let Some(pool) = layer.pool.as_mut() else {
                continue;
            };

            let Some(fractional_scale) = layer.fractional_scale else {
                continue;
            };

            let Some((width, height)) = layer.size else {
                continue;
            };

            let width = width * fractional_scale / 120;
            let height = height * fractional_scale / 120;

            if cur_resized_img
                .as_ref()
                .is_none_or(|img| img.width() != width || img.height() != height)
            {
                let Some(source) = self.current_source.as_ref() else {
                    tracing::info!("No source for wallpaper");
                    continue;
                };

                cur_resized_img = match source {
                    Source::Path(path) => {
                        if self.current_image.is_none() {
                            self.current_image = match ImageReader::open(path)
                                .ok()
                                .and_then(|f| f.with_guessed_format().ok())
                            {
                                Some(f) => match decode(f) {
                                    Ok(img) => Some(img),
                                    Err(why) => {
                                        tracing::warn!(
                                            ?why,
                                            "Failed to decode image: {}",
                                            path.display()
                                        );
                                        continue;
                                    }
                                },
                                None => continue,
                            };
                        }
                        let img = self.current_image.as_ref().unwrap();

                        match self.entry.scaling_mode {
                            ScalingMode::Fit(color) => Some(crate::scaler::fit(
                                img,
                                &color,
                                width,
                                height,
                                &self.entry.filter_method,
                            )),

                            ScalingMode::Zoom => Some(crate::scaler::zoom(
                                img,
                                width,
                                height,
                                &self.entry.filter_method,
                            )),

                            ScalingMode::Stretch => Some(crate::scaler::stretch(
                                img,
                                width,
                                height,
                                &self.entry.filter_method,
                            )),
                        }
                    }

                    Source::Color(Color::Single([r, g, b])) => Some(image::DynamicImage::from(
                        crate::colored::single([*r, *g, *b], width, height),
                    )),

                    Source::Color(Color::Gradient(gradient)) => {
                        match crate::colored::gradient(gradient, width, height) {
                            Ok(buffer) => Some(image::DynamicImage::from(buffer)),
                            Err(why) => {
                                tracing::error!(
                                    ?gradient,
                                    ?why,
                                    "color gradient in config is invalid"
                                );
                                // Deliberate divergence: upstream leaves None here and
                                // unwraps it below, which panics. Keep the `continue`.
                                continue;
                            }
                        }
                    }
                };
            }

            let image = cur_resized_img.as_ref().unwrap();
            let buffer_result =
                crate::draw::canvas(pool, image, width as i32, height as i32, width as i32 * 4);

            match buffer_result {
                Ok(buffer) => {
                    crate::draw::layer_surface(
                        layer,
                        &self.queue_handle,
                        &buffer,
                        (width as i32, height as i32),
                    );
                    layer.needs_redraw = false;

                    let elapsed = Instant::now().duration_since(start);

                    tracing::debug!(?elapsed, source = ?self.entry.source, "wallpaper draw");
                }

                Err(why) => {
                    tracing::error!(?why, "wallpaper could not be drawn");
                }
            }
        }
    }

    pub fn load_images(&mut self) {
        let mut image_queue = VecDeque::new();

        match self.entry.source {
            Source::Path(ref source) => {
                tracing::debug!(?source, "loading images");

                image_queue = collect_image_paths(source);

                if image_queue.len() > 1 {
                    let image_slice = image_queue.make_contiguous();
                    match self.entry.sampling_method {
                        SamplingMethod::Alphanumeric => {
                            image_slice
                                .sort_by(|a, b| a.to_string_lossy().cmp(&b.to_string_lossy()));
                        }
                        SamplingMethod::Random => image_slice.shuffle(&mut rng()),
                    };

                    // If a wallpaper from this slideshow was previously set, resume with that wallpaper.
                    if let Some(Source::Path(last_path)) = current_image(&self.entry.output)
                        && image_queue.contains(&last_path)
                    {
                        while let Some(path) = image_queue.pop_front() {
                            if path == last_path {
                                image_queue.push_front(path);
                                break;
                            }

                            image_queue.push_back(path);
                        }
                    }
                }

                if let Some(current_image_path) = image_queue.pop_front() {
                    self.current_source = Some(Source::Path(current_image_path.clone()));
                    image_queue.push_back(current_image_path);
                }
            }

            Source::Color(ref c) => {
                self.current_source = Some(Source::Color(c.clone()));
            }
        };
        if let Err(err) = self.save_state() {
            error!("{err}");
        }
        self.image_queue = image_queue;
    }

    fn watch_source(&mut self, tx: calloop::channel::SyncSender<(String, notify::Event)>) {
        let Source::Path(ref source) = self.entry.source else {
            return;
        };

        // Watch the canonicalized path: `load_images` canonicalizes before walking,
        // so event paths must share the prefix of the paths stored in the queue.
        let source = source.canonicalize().unwrap_or_else(|_| source.clone());

        let output = self.entry.output.clone();
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                if let Ok(e) = res {
                    let _ = tx.send((output.clone(), e));
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(why) => {
                tracing::warn!(?why, "failed to create a watcher for the wallpaper source");
                return;
            }
        };

        tracing::debug!(output = self.entry.output, "watching source");

        if let Ok(m) = fs::metadata(&source) {
            if m.is_dir() {
                let _ = watcher.watch(&source, RecursiveMode::Recursive);
            } else if m.is_file() {
                let _ = watcher.watch(&source, RecursiveMode::NonRecursive);
            }
        } else {
            tracing::warn!(
                source = %source.display(),
                "wallpaper source does not exist; changes will not be watched"
            );
        }

        self.watcher = Some(watcher);
    }

    fn register_timer(&mut self) {
        let rotation_freq = self.entry.rotation_frequency;
        let cosmic_bg_clone = self.entry.output.clone();
        // set timer for rotation
        if rotation_freq > 0 {
            self.timer_token = self
                .loop_handle
                .insert_source(
                    Timer::from_duration(Duration::from_secs(rotation_freq)),
                    move |_, _, state: &mut CosmicBg| {
                        let span = tracing::debug_span!("Wallpaper::timer");
                        let _handle = span.enter();

                        let Some(item) = state
                            .wallpapers
                            .iter_mut()
                            .find(|w| w.entry.output == cosmic_bg_clone)
                        else {
                            return TimeoutAction::Drop; // Drop if no item found for this timer
                        };

                        if let Some(next) = item.image_queue.pop_front() {
                            item.current_source = Some(Source::Path(next.clone()));
                            if let Err(err) = item.save_state() {
                                error!("{err}");
                            }

                            item.image_queue.push_back(next);
                            item.clear_image();
                            item.draw();

                            return TimeoutAction::ToDuration(Duration::from_secs(rotation_freq));
                        }

                        // The timer source is dropped; clear the stale token so
                        // `ensure_active` can register a new timer later.
                        item.timer_token = None;
                        TimeoutAction::Drop
                    },
                )
                .ok();
        }
    }

    fn clear_image(&mut self) {
        self.current_image = None;
        for l in &mut self.layers {
            l.needs_redraw = true;
        }
    }

    /// Displays a queued image and restarts the rotation timer after files
    /// arrive in a source that was empty, or whose timer already dropped.
    pub fn ensure_active(&mut self) {
        if self.current_source.is_none()
            && let Some(next) = self.image_queue.pop_front()
        {
            self.current_source = Some(Source::Path(next.clone()));
            if let Err(err) = self.save_state() {
                error!("{err}");
            }

            self.image_queue.push_back(next);
            self.clear_image();
            self.draw();
        }

        if self.timer_token.is_none() {
            self.register_timer();
        }
    }
}

fn collect_image_paths(source: &Path) -> VecDeque<PathBuf> {
    let mut image_queue = VecDeque::new();

    if let Ok(source) = source.canonicalize() {
        if source.is_dir() {
            // Recursively collect images in the directory for the slideshow.
            for img_path in WalkDir::new(source)
                .follow_links(true)
                .into_iter()
                .filter_map(Result::ok)
                .filter(|p| p.path().is_file())
            {
                image_queue.push_front(img_path.path().into());
            }
        } else if source.is_file() {
            image_queue.push_front(source);
        }
    }

    image_queue
}

fn decode(mut reader: ImageReader<BufReader<fs::File>>) -> ImageResult<DynamicImage> {
    let mut limits = Limits::default();
    limits.max_alloc = Some(1024 * 1024 * 1024);
    reader.limits(limits.clone());

    let mut decoder = reader.into_decoder()?;
    let orientation = decoder.orientation()?;

    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;

    let mut image = DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);

    Ok(image)
}

fn current_image(output: &str) -> Option<Source> {
    let state = State::state().ok()?;
    let mut wallpapers = State::get_entry(&state)
        .unwrap_or_default()
        .wallpapers
        .into_iter();

    let wallpaper = if output == "all" {
        wallpapers.next()
    } else {
        wallpapers.into_iter().find(|(name, _path)| name == output)
    };

    wallpaper.map(|(_name, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[test]
    fn test_custom_dir_loading() {
        // Create a temp directory structure
        // root/
        //   img1.png
        //   subdir/
        //     img2.png

        let dir = tempdir().unwrap();
        let root = dir.path();
        let subdir = root.join("subdir");
        fs::create_dir(&subdir).unwrap();

        File::create(root.join("img1.png")).unwrap();
        File::create(subdir.join("img2.png")).unwrap();

        let image_queue = collect_image_paths(root);

        assert_eq!(image_queue.len(), 2, "Should find 2 images recursively");
        assert!(
            image_queue
                .iter()
                .any(|p: &PathBuf| p.ends_with("img1.png"))
        );
        assert!(
            image_queue
                .iter()
                .any(|p: &PathBuf| p.ends_with("img2.png"))
        );
    }

    #[test]
    fn test_single_file_loading() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("img1.png");
        File::create(&file).unwrap();

        let image_queue = collect_image_paths(&file);

        assert_eq!(image_queue.len(), 1);
        assert!(image_queue[0].ends_with("img1.png"));
    }

    #[test]
    fn test_decode_applies_exif_orientation() {
        // Minimal EXIF APP1 segment: "Exif\0\0" identifier, a little-endian TIFF
        // header, and an IFD0 with the single Orientation (0x0112) tag set to 6
        // (rotate 90 degrees clockwise), which swaps width and height.
        const EXIF_APP1: [u8; 36] = [
            0xFF, 0xE1, // APP1 marker
            0x00, 0x22, // segment length (34, includes these two bytes)
            b'E', b'x', b'i', b'f', 0x00, 0x00, // Exif identifier
            0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, // TIFF header, IFD0 at offset 8
            0x01, 0x00, // one IFD0 entry
            0x12, 0x01, 0x03, 0x00, 0x01, 0x00, 0x00, 0x00, // Orientation, SHORT, count 1
            0x06, 0x00, 0x00, 0x00, // value 6
            0x00, 0x00, 0x00, 0x00, // no next IFD
        ];

        let mut jpeg = Vec::new();
        DynamicImage::ImageRgb8(image::RgbImage::new(2, 1))
            .write_to(
                &mut std::io::Cursor::new(&mut jpeg),
                image::ImageFormat::Jpeg,
            )
            .unwrap();

        // Splice the APP1 segment right after the SOI marker.
        let mut data = jpeg[..2].to_vec();
        data.extend_from_slice(&EXIF_APP1);
        data.extend_from_slice(&jpeg[2..]);

        let dir = tempdir().unwrap();
        let path = dir.path().join("oriented.jpg");
        fs::write(&path, data).unwrap();

        let reader = ImageReader::open(&path)
            .unwrap()
            .with_guessed_format()
            .unwrap();
        let decoded = decode(reader).unwrap();

        assert_eq!(
            (decoded.width(), decoded.height()),
            (1, 2),
            "orientation 6 should swap the 2x1 dimensions"
        );
    }
}
