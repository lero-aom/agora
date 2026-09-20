use std::{env, error::Error, fs, path::PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use image::ImageEncoder;
use url::Url;

fn main() -> Result<(), Box<dyn Error>> {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR")?);
    let logo_png = manifest_dir.join("../../assets/logo/logo.png");
    println!("cargo:rerun-if-changed={}", logo_png.display());
    println!("cargo:rerun-if-env-changed=AGORA_UPDATE_BASE_URL");
    println!("cargo:rerun-if-env-changed=AGORA_UPDATE_PUBLIC_KEY_B64");

    write_update_config()?;

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

fn write_update_config() -> Result<(), Box<dyn Error>> {
    let base_url = env::var("AGORA_UPDATE_BASE_URL").ok();
    let public_key = env::var("AGORA_UPDATE_PUBLIC_KEY_B64").ok();

    let configured_values = [&base_url, &public_key]
        .iter()
        .filter(|value| value.is_some())
        .count();
    if configured_values != 0 && configured_values != 2 {
        return Err(
            "AGORA_UPDATE_BASE_URL and AGORA_UPDATE_PUBLIC_KEY_B64 must be set together".into(),
        );
    }

    let (enabled, base_url, public_key) = if configured_values == 0 {
        (false, String::new(), String::new())
    } else {
        let base_url = base_url.expect("checked above").trim().to_string();
        let public_key = public_key.expect("checked above").trim().to_string();

        validate_https_url(&base_url, "AGORA_UPDATE_BASE_URL", true)?;
        let decoded_key = STANDARD
            .decode(&public_key)
            .map_err(|_| "AGORA_UPDATE_PUBLIC_KEY_B64 must be canonical Base64")?;
        if decoded_key.len() != 32 || STANDARD.encode(decoded_key) != public_key {
            return Err("AGORA_UPDATE_PUBLIC_KEY_B64 must encode exactly 32 bytes".into());
        }

        (true, base_url, public_key)
    };

    let target = env::var("TARGET")?;
    let config = format!(
        "pub(crate) const UPDATE_ENABLED: bool = {enabled};\n\
         pub(crate) const UPDATE_BASE_URL: &str = {base_url:?};\n\
         pub(crate) const UPDATE_PUBLIC_KEY_B64: &str = {public_key:?};\n\
         pub(crate) const UPDATE_TARGET: &str = {target:?};\n"
    );
    fs::write(
        PathBuf::from(env::var("OUT_DIR")?).join("agora_update_config.rs"),
        config,
    )?;
    Ok(())
}

fn validate_https_url(
    value: &str,
    variable: &str,
    require_trailing_slash: bool,
) -> Result<(), Box<dyn Error>> {
    let url = Url::parse(value).map_err(|_| format!("{variable} must be an absolute HTTPS URL"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(format!(
            "{variable} must be an absolute HTTPS URL without credentials, query, or fragment"
        )
        .into());
    }
    if require_trailing_slash && !value.ends_with('/') {
        return Err(format!(
            "{variable} must end with '/' so update asset paths remain pinned below it"
        )
        .into());
    }
    Ok(())
}
