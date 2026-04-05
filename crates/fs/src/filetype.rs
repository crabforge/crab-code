//! File type detection based on extension and magic bytes.
//!
//! Used by tools to decide whether a file is text or binary, and to
//! provide appropriate handling (e.g., skip binary files in grep,
//! prevent writing binary content as text).

use std::path::Path;

// ── Public types ──────────────────────────────────────────────────────

/// Broad category of a file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FileCategory {
    /// Source code or plain text.
    Text,
    /// Binary data (images, executables, etc.).
    Binary,
    /// Image file (subset of Binary with known format).
    Image,
    /// Document (PDF, Office, etc.).
    Document,
    /// Unknown or unrecognized.
    Unknown,
}

impl std::fmt::Display for FileCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Text => f.write_str("text"),
            Self::Binary => f.write_str("binary"),
            Self::Image => f.write_str("image"),
            Self::Document => f.write_str("document"),
            Self::Unknown => f.write_str("unknown"),
        }
    }
}

/// Detected file type information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileType {
    /// The broad category.
    pub category: FileCategory,
    /// MIME type guess (e.g., "text/plain", "image/png").
    pub mime: String,
    /// File extension (lowercase, without dot), if any.
    pub extension: Option<String>,
}

// ── Extension-based detection ─────────────────────────────────────────

/// Detect file type based on the file extension alone.
#[must_use]
pub fn detect_by_extension(path: &Path) -> FileType {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_lowercase);

    let (category, mime) = match ext.as_deref() {
        // Source code
        Some("rs") => (FileCategory::Text, "text/x-rust"),
        Some("py") => (FileCategory::Text, "text/x-python"),
        Some("js" | "mjs" | "cjs") => (FileCategory::Text, "text/javascript"),
        Some("ts" | "mts" | "cts") => (FileCategory::Text, "text/typescript"),
        Some("go") => (FileCategory::Text, "text/x-go"),
        Some("java") => (FileCategory::Text, "text/x-java"),
        Some("c" | "h") => (FileCategory::Text, "text/x-c"),
        Some("cpp" | "cc" | "cxx" | "hpp") => (FileCategory::Text, "text/x-c++"),
        Some("rb") => (FileCategory::Text, "text/x-ruby"),
        Some("sh" | "bash" | "zsh") => (FileCategory::Text, "text/x-shellscript"),
        Some("lua") => (FileCategory::Text, "text/x-lua"),
        Some("zig") => (FileCategory::Text, "text/x-zig"),

        // Config / data
        Some("json") => (FileCategory::Text, "application/json"),
        Some("toml") => (FileCategory::Text, "application/toml"),
        Some("yaml" | "yml") => (FileCategory::Text, "application/yaml"),
        Some("xml") => (FileCategory::Text, "application/xml"),
        Some("csv") => (FileCategory::Text, "text/csv"),
        Some("ini" | "cfg" | "conf" | "txt" | "text" | "log") => (FileCategory::Text, "text/plain"),

        // Web
        Some("html" | "htm") => (FileCategory::Text, "text/html"),
        Some("css") => (FileCategory::Text, "text/css"),
        Some("svg") => (FileCategory::Text, "image/svg+xml"),

        // Documentation
        Some("md" | "markdown") => (FileCategory::Text, "text/markdown"),
        Some("rst") => (FileCategory::Text, "text/x-rst"),

        // Build / CI
        Some("dockerfile") => (FileCategory::Text, "text/x-dockerfile"),
        Some("makefile") => (FileCategory::Text, "text/x-makefile"),

        // Images
        Some("png") => (FileCategory::Image, "image/png"),
        Some("jpg" | "jpeg") => (FileCategory::Image, "image/jpeg"),
        Some("gif") => (FileCategory::Image, "image/gif"),
        Some("bmp") => (FileCategory::Image, "image/bmp"),
        Some("webp") => (FileCategory::Image, "image/webp"),
        Some("ico") => (FileCategory::Image, "image/x-icon"),
        Some("tiff" | "tif") => (FileCategory::Image, "image/tiff"),

        // Documents
        Some("pdf") => (FileCategory::Document, "application/pdf"),
        Some("docx") => (
            FileCategory::Document,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        Some("xlsx") => (
            FileCategory::Document,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        Some("pptx") => (
            FileCategory::Document,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),

        // Binary
        Some("exe" | "dll" | "so" | "dylib" | "o" | "obj") => {
            (FileCategory::Binary, "application/octet-stream")
        }
        Some("wasm") => (FileCategory::Binary, "application/wasm"),
        Some("zip") => (FileCategory::Binary, "application/zip"),
        Some("gz" | "tgz") => (FileCategory::Binary, "application/gzip"),
        Some("tar") => (FileCategory::Binary, "application/x-tar"),

        _ => (FileCategory::Unknown, "application/octet-stream"),
    };

    FileType {
        category,
        mime: mime.to_string(),
        extension: ext,
    }
}

/// Detect file type by reading the first few bytes (magic bytes).
///
/// Falls back to extension-based detection if magic bytes are
/// unrecognized. Returns `Unknown` if the file cannot be read.
#[must_use]
pub fn detect_by_magic(path: &Path) -> FileType {
    let Ok(header) = std::fs::read(path) else {
        return detect_by_extension(path);
    };

    if let Some(ft) = detect_magic_bytes(&header) {
        return ft;
    }

    // If magic bytes are inconclusive, check if it looks like text
    if is_likely_text(&header) {
        let mut ft = detect_by_extension(path);
        if ft.category == FileCategory::Unknown {
            ft.category = FileCategory::Text;
            ft.mime = "text/plain".to_string();
        }
        return ft;
    }

    // Fall back to extension
    let mut ft = detect_by_extension(path);
    if ft.category == FileCategory::Unknown {
        ft.category = FileCategory::Binary;
    }
    ft
}

/// Check a byte slice against known magic byte signatures.
fn detect_magic_bytes(data: &[u8]) -> Option<FileType> {
    if data.len() < 4 {
        return None;
    }

    // PNG: 89 50 4E 47
    if data.starts_with(&[0x89, 0x50, 0x4E, 0x47]) {
        return Some(FileType {
            category: FileCategory::Image,
            mime: "image/png".to_string(),
            extension: Some("png".to_string()),
        });
    }

    // JPEG: FF D8 FF
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some(FileType {
            category: FileCategory::Image,
            mime: "image/jpeg".to_string(),
            extension: Some("jpg".to_string()),
        });
    }

    // GIF: 47 49 46 38
    if data.starts_with(&[0x47, 0x49, 0x46, 0x38]) {
        return Some(FileType {
            category: FileCategory::Image,
            mime: "image/gif".to_string(),
            extension: Some("gif".to_string()),
        });
    }

    // PDF: 25 50 44 46 (%PDF)
    if data.starts_with(&[0x25, 0x50, 0x44, 0x46]) {
        return Some(FileType {
            category: FileCategory::Document,
            mime: "application/pdf".to_string(),
            extension: Some("pdf".to_string()),
        });
    }

    // ZIP (also docx/xlsx/pptx): 50 4B 03 04
    if data.starts_with(&[0x50, 0x4B, 0x03, 0x04]) {
        return Some(FileType {
            category: FileCategory::Binary,
            mime: "application/zip".to_string(),
            extension: Some("zip".to_string()),
        });
    }

    // ELF binary: 7F 45 4C 46
    if data.starts_with(&[0x7F, 0x45, 0x4C, 0x46]) {
        return Some(FileType {
            category: FileCategory::Binary,
            mime: "application/x-elf".to_string(),
            extension: None,
        });
    }

    // WASM: 00 61 73 6D
    if data.starts_with(&[0x00, 0x61, 0x73, 0x6D]) {
        return Some(FileType {
            category: FileCategory::Binary,
            mime: "application/wasm".to_string(),
            extension: Some("wasm".to_string()),
        });
    }

    // Gzip: 1F 8B
    if data.len() >= 2 && data[0] == 0x1F && data[1] == 0x8B {
        return Some(FileType {
            category: FileCategory::Binary,
            mime: "application/gzip".to_string(),
            extension: Some("gz".to_string()),
        });
    }

    // BMP: 42 4D
    if data.len() >= 2 && data[0] == 0x42 && data[1] == 0x4D {
        return Some(FileType {
            category: FileCategory::Image,
            mime: "image/bmp".to_string(),
            extension: Some("bmp".to_string()),
        });
    }

    // WebP: RIFF....WEBP
    if data.len() >= 12 && data.starts_with(b"RIFF") && &data[8..12] == b"WEBP" {
        return Some(FileType {
            category: FileCategory::Image,
            mime: "image/webp".to_string(),
            extension: Some("webp".to_string()),
        });
    }

    None
}

/// Heuristic: check if data looks like text (no null bytes in first 8KB,
/// mostly printable ASCII or valid UTF-8).
fn is_likely_text(data: &[u8]) -> bool {
    let check_len = data.len().min(8192);
    let sample = &data[..check_len];

    // Null byte = almost certainly binary
    if sample.contains(&0) {
        return false;
    }

    // If valid UTF-8, it's text
    std::str::from_utf8(sample).is_ok()
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Extension detection ───────────────────────────────────────

    #[test]
    fn detect_rust_file() {
        let ft = detect_by_extension(Path::new("src/main.rs"));
        assert_eq!(ft.category, FileCategory::Text);
        assert_eq!(ft.mime, "text/x-rust");
        assert_eq!(ft.extension.as_deref(), Some("rs"));
    }

    #[test]
    fn detect_python_file() {
        let ft = detect_by_extension(Path::new("script.py"));
        assert_eq!(ft.category, FileCategory::Text);
        assert_eq!(ft.mime, "text/x-python");
    }

    #[test]
    fn detect_png_by_extension() {
        let ft = detect_by_extension(Path::new("logo.png"));
        assert_eq!(ft.category, FileCategory::Image);
        assert_eq!(ft.mime, "image/png");
    }

    #[test]
    fn detect_pdf_by_extension() {
        let ft = detect_by_extension(Path::new("doc.pdf"));
        assert_eq!(ft.category, FileCategory::Document);
    }

    #[test]
    fn detect_exe_by_extension() {
        let ft = detect_by_extension(Path::new("app.exe"));
        assert_eq!(ft.category, FileCategory::Binary);
    }

    #[test]
    fn detect_unknown_extension() {
        let ft = detect_by_extension(Path::new("file.xyz123"));
        assert_eq!(ft.category, FileCategory::Unknown);
    }

    #[test]
    fn detect_no_extension() {
        let ft = detect_by_extension(Path::new("Makefile"));
        assert_eq!(ft.category, FileCategory::Unknown);
    }

    #[test]
    fn detect_json_file() {
        let ft = detect_by_extension(Path::new("config.json"));
        assert_eq!(ft.category, FileCategory::Text);
        assert_eq!(ft.mime, "application/json");
    }

    #[test]
    fn detect_toml_file() {
        let ft = detect_by_extension(Path::new("Cargo.toml"));
        assert_eq!(ft.category, FileCategory::Text);
        assert_eq!(ft.mime, "application/toml");
    }

    #[test]
    fn case_insensitive_extension() {
        let ft = detect_by_extension(Path::new("photo.PNG"));
        assert_eq!(ft.category, FileCategory::Image);
    }

    // ── Magic bytes detection ─────────────────────────────────────

    #[test]
    fn magic_png() {
        let ft = detect_magic_bytes(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(ft.unwrap().category, FileCategory::Image);
    }

    #[test]
    fn magic_jpeg() {
        let ft = detect_magic_bytes(&[0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        assert_eq!(ft.category, FileCategory::Image);
        assert_eq!(ft.mime, "image/jpeg");
    }

    #[test]
    fn magic_gif() {
        let ft = detect_magic_bytes(&[0x47, 0x49, 0x46, 0x38, 0x39, 0x61]);
        assert_eq!(ft.unwrap().category, FileCategory::Image);
    }

    #[test]
    fn magic_pdf() {
        let ft = detect_magic_bytes(b"%PDF-1.7");
        assert_eq!(ft.unwrap().category, FileCategory::Document);
    }

    #[test]
    fn magic_zip() {
        let ft = detect_magic_bytes(&[0x50, 0x4B, 0x03, 0x04, 0x00]);
        assert_eq!(ft.unwrap().category, FileCategory::Binary);
    }

    #[test]
    fn magic_elf() {
        let ft = detect_magic_bytes(&[0x7F, 0x45, 0x4C, 0x46, 0x02]);
        assert_eq!(ft.unwrap().category, FileCategory::Binary);
    }

    #[test]
    fn magic_wasm() {
        let ft = detect_magic_bytes(&[0x00, 0x61, 0x73, 0x6D, 0x01]).unwrap();
        assert_eq!(ft.category, FileCategory::Binary);
        assert_eq!(ft.mime, "application/wasm");
    }

    #[test]
    fn magic_gzip() {
        let ft = detect_magic_bytes(&[0x1F, 0x8B, 0x08, 0x00]);
        assert_eq!(ft.unwrap().category, FileCategory::Binary);
    }

    #[test]
    fn magic_bmp() {
        let ft = detect_magic_bytes(&[0x42, 0x4D, 0x00, 0x00, 0x00]);
        assert_eq!(ft.unwrap().category, FileCategory::Image);
    }

    #[test]
    fn magic_unknown() {
        let ft = detect_magic_bytes(&[0x01, 0x02, 0x03, 0x04]);
        assert!(ft.is_none());
    }

    #[test]
    fn magic_too_short() {
        let ft = detect_magic_bytes(&[0x89, 0x50]);
        assert!(ft.is_none());
    }

    // ── Text heuristic ────────────────────────────────────────────

    #[test]
    fn text_heuristic_valid_utf8() {
        assert!(is_likely_text(b"Hello, world!\nLine 2\n"));
    }

    #[test]
    fn text_heuristic_null_byte_is_binary() {
        assert!(!is_likely_text(b"Hello\x00world"));
    }

    #[test]
    fn text_heuristic_empty_is_text() {
        assert!(is_likely_text(b""));
    }

    // ── Full detection with file ──────────────────────────────────

    #[test]
    fn detect_text_file_by_magic() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.txt");
        fs::write(&file, "Hello, world!").unwrap();
        let ft = detect_by_magic(&file);
        assert_eq!(ft.category, FileCategory::Text);
    }

    #[test]
    fn detect_binary_file_by_magic() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.bin");
        // Write PNG header
        fs::write(&file, [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]).unwrap();
        let ft = detect_by_magic(&file);
        assert_eq!(ft.category, FileCategory::Image);
        assert_eq!(ft.mime, "image/png");
    }

    #[test]
    fn detect_nonexistent_falls_to_extension() {
        let ft = detect_by_magic(Path::new("/nonexistent/file.rs"));
        assert_eq!(ft.category, FileCategory::Text);
        assert_eq!(ft.mime, "text/x-rust");
    }

    // ── Display ───────────────────────────────────────────────────

    #[test]
    fn category_display() {
        assert_eq!(FileCategory::Text.to_string(), "text");
        assert_eq!(FileCategory::Binary.to_string(), "binary");
        assert_eq!(FileCategory::Image.to_string(), "image");
        assert_eq!(FileCategory::Document.to_string(), "document");
        assert_eq!(FileCategory::Unknown.to_string(), "unknown");
    }
}
