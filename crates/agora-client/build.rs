use std::{env, error::Error, fs, path::PathBuf};

use image::ImageEncoder;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let logo_png = manifest_dir.join("../../assets/logo/logo.png");
    println!("cargo:rerun-if-changed={}", logo_png.display());

    if env::var("CARGO_CFG_TARGET_OS").unwrap_or_default() != "windows" {
        return Ok(());
    }

    let image = image::load_from_memory(&fs::read(&logo_png)?)?
        .resize(256, 256, image::imageops::FilterType::Lanczos3)
        .into_rgba8();
    let (width, height) = image.dimensions();

    let icon_path = PathBuf::from(env::var("OUT_DIR")?).join("agora.ico");
    let icon_file = fs::File::create(&icon_path)?;
    image::codecs::ico::IcoEncoder::new(icon_file).write_image(
        image.as_raw(),
        width,
        height,
        image::ColorType::Rgba8.into(),
    )?;

    let icon_path = icon_path.to_string_lossy().replace('\\', "/");
    winresource::WindowsResource::new()
        .set_icon(&icon_path)
        .compile()?;

    Ok(())
}
