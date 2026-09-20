use std::{env, fs, path::Path};

use base64::{engine::general_purpose::STANDARD, Engine as _};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};

const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const SIGNATURE_BASE64_LEN: usize = 88;

fn main() {
    if cfg!(debug_assertions) {
        fail("agora-update-sign only runs from a release build; use cargo run --release");
    }

    let arguments = env::args().skip(1).collect::<Vec<_>>();
    let result = match arguments.as_slice() {
        [command] if command == "public-key" => print_public_key(),
        [command, manifest_flag, manifest, output_flag, output]
            if command == "sign" && manifest_flag == "--manifest" && output_flag == "--output" =>
        {
            sign_manifest(Path::new(manifest), Path::new(output))
        }
        [command, manifest_flag, manifest, signature_flag, signature, public_key_flag, public_key]
            if command == "verify"
                && manifest_flag == "--manifest"
                && signature_flag == "--signature"
                && public_key_flag == "--public-key" =>
        {
            verify_manifest(Path::new(manifest), Path::new(signature), public_key)
        }
        _ => Err(
            "usage: agora-update-sign public-key | sign --manifest <path> --output <path> | verify --manifest <path> --signature <path> --public-key <base64>".to_string(),
        ),
    };

    if let Err(error) = result {
        fail(&error);
    }
}

fn print_public_key() -> Result<(), String> {
    let signing_key = signing_key_from_environment()?;
    println!(
        "{}",
        STANDARD.encode(signing_key.verifying_key().as_bytes())
    );
    Ok(())
}

fn sign_manifest(manifest_path: &Path, signature_path: &Path) -> Result<(), String> {
    let manifest = read_manifest(manifest_path)?;
    let signing_key = signing_key_from_environment()?;
    let signature = signing_key.sign(&manifest);
    fs::write(signature_path, STANDARD.encode(signature.to_bytes()))
        .map_err(|error| format!("could not write detached signature: {error}"))
}

fn verify_manifest(
    manifest_path: &Path,
    signature_path: &Path,
    public_key: &str,
) -> Result<(), String> {
    let manifest = read_manifest(manifest_path)?;
    let encoded_signature = fs::read_to_string(signature_path)
        .map_err(|error| format!("could not read detached signature: {error}"))?;
    if encoded_signature.len() != SIGNATURE_BASE64_LEN {
        return Err("detached signature must be canonical Base64".to_string());
    }
    let decoded_signature = STANDARD
        .decode(&encoded_signature)
        .map_err(|_| "detached signature must be canonical Base64".to_string())?;
    if decoded_signature.len() != 64 || STANDARD.encode(&decoded_signature) != encoded_signature {
        return Err("detached signature must be canonical Base64".to_string());
    }
    let signature = Signature::from_bytes(
        &decoded_signature
            .as_slice()
            .try_into()
            .map_err(|_| "detached signature has an invalid length".to_string())?,
    );
    let verifying_key = verifying_key_from_base64(public_key)?;
    verifying_key
        .verify_strict(&manifest, &signature)
        .map_err(|_| "detached signature does not match the manifest".to_string())
}

fn read_manifest(manifest_path: &Path) -> Result<Vec<u8>, String> {
    let metadata = fs::metadata(manifest_path)
        .map_err(|error| format!("could not inspect manifest: {error}"))?;
    if metadata.len() > MAX_MANIFEST_BYTES {
        return Err(format!(
            "manifest exceeds the {MAX_MANIFEST_BYTES}-byte limit"
        ));
    }
    fs::read(manifest_path).map_err(|error| format!("could not read manifest: {error}"))
}

fn signing_key_from_environment() -> Result<SigningKey, String> {
    let encoded = env::var("AGORA_UPDATE_SIGNING_KEY_B64")
        .map_err(|_| "AGORA_UPDATE_SIGNING_KEY_B64 is required".to_string())?;
    let mut decoded = STANDARD
        .decode(&encoded)
        .map_err(|_| "AGORA_UPDATE_SIGNING_KEY_B64 must be canonical Base64".to_string())?;
    if decoded.len() != 32 || STANDARD.encode(&decoded) != encoded {
        return Err("AGORA_UPDATE_SIGNING_KEY_B64 must encode exactly 32 seed bytes".to_string());
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&decoded);
    decoded.fill(0);
    let signing_key = SigningKey::from_bytes(&bytes);
    bytes.fill(0);
    Ok(signing_key)
}

fn verifying_key_from_base64(encoded: &str) -> Result<VerifyingKey, String> {
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| "public key must be canonical Base64".to_string())?;
    if decoded.len() != 32 || STANDARD.encode(&decoded) != encoded {
        return Err("public key must be canonical Base64".to_string());
    }
    let bytes: [u8; 32] = decoded
        .as_slice()
        .try_into()
        .map_err(|_| "public key has an invalid length".to_string())?;
    VerifyingKey::from_bytes(&bytes).map_err(|_| "public key is invalid".to_string())
}

fn fail(message: &str) -> ! {
    eprintln!("agora-update-sign: {message}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn verifies_a_detached_manifest_signature() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let manifest = b"{\"schema_version\":1}";
        let suffix = format!(
            "agora-update-sign-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let manifest_path = std::env::temp_dir().join(format!("{suffix}.json"));
        let signature_path = std::env::temp_dir().join(format!("{suffix}.sig"));
        fs::write(&manifest_path, manifest).unwrap();
        fs::write(
            &signature_path,
            STANDARD.encode(signing_key.sign(manifest).to_bytes()),
        )
        .unwrap();

        let result = verify_manifest(
            &manifest_path,
            &signature_path,
            &STANDARD.encode(signing_key.verifying_key().as_bytes()),
        );
        let _ = fs::remove_file(&manifest_path);
        let _ = fs::remove_file(&signature_path);

        assert!(result.is_ok());
    }
}
