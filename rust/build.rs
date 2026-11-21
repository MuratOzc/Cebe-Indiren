use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Cursor, Read};
use std::path::{Path, PathBuf};

// Pinned third-party tools. To upgrade, change the URL and the hashes together; the build refuses
// anything that does not match.
const YTDLP_VERSION: &str = "2026.10.26";
const YTDLP_SHA256: &str = "f5389d3698ecde4ef06f4e6b1ce0294fe9ac1396228c8990b8d3416d6d177a7d";
const YTDLP_ZIPAPP_SHA256: &str = "e51e69e89489f199530f13d32a4342da3ddb2cf34842f364603a066db05afbf3";
const FFMPEG_ZIP: (&str, &str) = (
    "https://github.com/GyanD/codexffmpeg/releases/download/9.1/ffmpeg-9.1-essentials_build.zip",
    "b9bbbb3acece000cbeec994f856649354e52c773807841e78908374b24d658fd",
);
const FFMPEG_MEMBERS: [(&str, &str, &str); 2] = [
    (
        "ffmpeg-9.1-essentials_build/bin/ffmpeg.exe",
        "ffmpeg.exe",
        "ed8f66cb877087a96f5bbb40a2944f0e764f2ad8870d5cb647360d912e62bc07",
    ),
    (
        "ffmpeg-9.1-essentials_build/bin/ffprobe.exe",
        "ffprobe.exe",
        "df0199f606faa88e9ad192732a7c3987acfb647663534c297772e17730611a08",
    ),
];

fn sha256_hex(mut reader: impl Read) -> String {
    let mut hasher = Sha256::new();
    std::io::copy(&mut reader, &mut hasher).unwrap();
    format!("{:x}", hasher.finalize())
}

fn is_valid(path: &Path, sha256: &str) -> bool {
    File::open(path).is_ok_and(|f| sha256_hex(BufReader::new(f)) == sha256)
}

fn download(url: &str, sha256: &str) -> Vec<u8> {
    println!("cargo:warning=Downloading {url}");
    let mut bytes = Vec::new();
    ureq::get(url)
        .call()
        .unwrap_or_else(|e| panic!("download failed for {url}: {e}"))
        .into_reader()
        .read_to_end(&mut bytes)
        .unwrap();
    let actual = sha256_hex(bytes.as_slice());
    assert_eq!(actual, sha256, "SHA-256 mismatch for {url}");
    bytes
}

fn write_verified(dest: &Path, bytes: &[u8], sha256: &str) {
    assert_eq!(sha256_hex(bytes), sha256, "SHA-256 mismatch for {}", dest.display());
    let tmp = dest.with_extension("exe.part");
    fs::write(&tmp, bytes).unwrap();
    fs::rename(&tmp, dest).unwrap();
}

fn ensure_bundled_binaries(dir: &Path) {
    fs::create_dir_all(dir).unwrap();

    let ytdlp = dir.join("yt-dlp.exe");
    if !is_valid(&ytdlp, YTDLP_SHA256) {
        let url = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{YTDLP_VERSION}/yt-dlp.exe");
        write_verified(&ytdlp, &download(&url, YTDLP_SHA256), YTDLP_SHA256);
    }

    if FFMPEG_MEMBERS.iter().any(|(_, name, sha)| !is_valid(&dir.join(name), sha)) {
        let zip = download(FFMPEG_ZIP.0, FFMPEG_ZIP.1);
        let mut archive = zip::ZipArchive::new(Cursor::new(zip)).unwrap();
        for (member, name, sha) in FFMPEG_MEMBERS {
            let mut bytes = Vec::new();
            archive.by_name(member).unwrap().read_to_end(&mut bytes).unwrap();
            write_verified(&dir.join(name), &bytes, sha);
        }
    }
}

fn ensure_ytdlp_zipapp(dir: &Path) {
    fs::create_dir_all(dir).unwrap();
    let ytdlp = dir.join("yt-dlp");
    if !is_valid(&ytdlp, YTDLP_ZIPAPP_SHA256) {
        let url = format!("https://github.com/yt-dlp/yt-dlp/releases/download/{YTDLP_VERSION}/yt-dlp");
        write_verified(&ytdlp, &download(&url, YTDLP_ZIPAPP_SHA256), YTDLP_ZIPAPP_SHA256);
    }
}

fn compress_into_out_dir(dir: &Path) {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    for name in ["yt-dlp.exe", "ffmpeg.exe", "ffprobe.exe"] {
        let input = BufReader::new(File::open(dir.join(name)).unwrap());
        let output = BufWriter::new(File::create(out_dir.join(format!("{name}.zst"))).unwrap());
        let mut encoder = zstd::Encoder::new(output, 19).unwrap();
        let threads = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
        encoder.multithread(threads).unwrap();
        encoder.long_distance_matching(true).unwrap();
        std::io::copy(&mut { input }, &mut encoder).unwrap();
        encoder.finish().unwrap();
    }
}

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    for name in ["yt-dlp.exe", "ffmpeg.exe", "ffprobe.exe", "yt-dlp"] {
        println!("cargo:rerun-if-changed=binaries/{name}");
    }

    println!("cargo:rustc-env=BUNDLED_YTDLP_VERSION={YTDLP_VERSION}");
    let dir = Path::new("binaries");
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => {
            ensure_bundled_binaries(dir);
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                compress_into_out_dir(dir);
            }
        }
        Ok("android") => ensure_ytdlp_zipapp(dir),
        _ => {}
    }
    tauri_build::build()
}
