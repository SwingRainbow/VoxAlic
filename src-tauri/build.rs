use flate2::write::GzEncoder;
use flate2::Compression;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    // Compress baro_zh.json → baro_zh_compressed.bin in OUT_DIR for embedded use.
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let json_path = PathBuf::from("resources/baro_zh.json");
    let json = std::fs::read(&json_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {}", json_path.display(), e));
    let mut gz = GzEncoder::new(Vec::new(), Compression::best());
    gz.write_all(&json).expect("gzip write");
    let compressed = gz.finish().expect("gzip finish");
    let dest = out_dir.join("baro_zh_compressed.bin");
    std::fs::write(&dest, &compressed)
        .unwrap_or_else(|e| panic!("failed to write {}: {}", dest.display(), e));

    // If smtp_creds.rs doesn't exist (fresh clone), copy from .example so the
    // build succeeds. The .example has dummy values — SMTP will fail auth but
    // the frontend gracefully falls back to QQ / clipboard.
    let creds_path = PathBuf::from("src/smtp_creds.rs");
    let example_path = PathBuf::from("src/smtp_creds.rs.example");
    if !creds_path.exists() && example_path.exists() {
        std::fs::copy(&example_path, &creds_path)
            .unwrap_or_else(|e| panic!("failed to copy {} → {}: {}", example_path.display(), creds_path.display(), e));
    }

    tauri_build::build()
}
