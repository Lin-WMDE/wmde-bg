// SPDX-License-Identifier: MPL-2.0

//! Background scaling methods such as fit, stretch, and zoom.

use image::imageops::FilterType;
use image::{DynamicImage, Pixel};
use wmde_bg_config::FilterMethod;

pub fn fit(
    img: &image::DynamicImage,
    color: &[f32; 3],
    layer_width: u32,
    layer_height: u32,
    filter: &FilterMethod,
) -> image::DynamicImage {
    // TODO: convert color to the same format as the input image.
    let mut filled_image =
        image::ImageBuffer::from_pixel(layer_width, layer_height, *image::Rgb::from_slice(color));

    let (w, h) = (img.width(), img.height());

    let ratio = (layer_width as f64 / w as f64).min(layer_height as f64 / h as f64);

    let (new_width, new_height) = (
        (w as f64 * ratio).round() as u32,
        (h as f64 * ratio).round() as u32,
    );

    let resized_image = resize(img, new_width, new_height, filter);

    image::imageops::replace(
        &mut filled_image,
        &resized_image.to_rgb32f(),
        ((layer_width - new_width) / 2).into(),
        ((layer_height - new_height) / 2).into(),
    );

    DynamicImage::from(filled_image)
}

pub fn stretch(
    img: &image::DynamicImage,
    layer_width: u32,
    layer_height: u32,
    filter: &FilterMethod,
) -> image::DynamicImage {
    resize(img, layer_width, layer_height, filter)
}

pub fn zoom(
    img: &image::DynamicImage,
    layer_width: u32,
    layer_height: u32,
    filter: &FilterMethod,
) -> image::DynamicImage {
    let (w, h) = (img.width(), img.height());

    let scale = (layer_width as f64 / w as f64).max(layer_height as f64 / h as f64);

    // Resize directly to the layer size with a source-side crop: scaling the
    // whole source first can allocate a huge intermediate image for extreme
    // aspect ratios.
    let crop_w = (layer_width as f64 / scale).min(w as f64);
    let crop_h = (layer_height as f64 / scale).min(h as f64);
    let left = (w as f64 - crop_w) / 2.0;
    let top = (h as f64 - crop_h) / 2.0;

    let mut resizer = fast_image_resize::Resizer::new();
    let options = fast_image_resize::ResizeOptions {
        algorithm: resize_alg(filter),
        cropping: fast_image_resize::SrcCropping::Crop(fast_image_resize::CropBox {
            left,
            top,
            width: crop_w,
            height: crop_h,
        }),
        ..Default::default()
    };
    let mut new_image = image::DynamicImage::new(layer_width, layer_height, img.color());
    if let Err(err) = resizer.resize(img, &mut new_image, &options) {
        tracing::warn!(?err, "Failed to use `fast_image_resize`. Falling back.");
        let crop_w = (crop_w.round() as u32).min(w).max(1);
        let crop_h = (crop_h.round() as u32).min(h).max(1);
        let cropped = img.crop_imm(
            w.saturating_sub(crop_w) / 2,
            h.saturating_sub(crop_h) / 2,
            crop_w,
            crop_h,
        );
        new_image = image::imageops::resize(
            &cropped,
            layer_width,
            layer_height,
            FilterType::from(filter.clone()),
        )
        .into();
    }
    new_image
}

fn resize(
    img: &image::DynamicImage,
    new_width: u32,
    new_height: u32,
    filter: &FilterMethod,
) -> image::DynamicImage {
    let mut resizer = fast_image_resize::Resizer::new();
    let options = fast_image_resize::ResizeOptions {
        algorithm: resize_alg(filter),
        ..Default::default()
    };
    let mut new_image = image::DynamicImage::new(new_width, new_height, img.color());
    if let Err(err) = resizer.resize(img, &mut new_image, &options) {
        tracing::warn!(?err, "Failed to use `fast_image_resize`. Falling back.");
        new_image =
            image::imageops::resize(img, new_width, new_height, FilterType::from(filter.clone()))
                .into();
    }
    new_image
}

fn resize_alg(filter: &FilterMethod) -> fast_image_resize::ResizeAlg {
    match filter {
        FilterMethod::Nearest => fast_image_resize::ResizeAlg::Nearest,
        FilterMethod::Linear => {
            fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Bilinear)
        }
        FilterMethod::Lanczos => {
            fast_image_resize::ResizeAlg::Convolution(fast_image_resize::FilterType::Lanczos3)
        }
    }
}
