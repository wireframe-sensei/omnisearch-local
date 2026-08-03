use calamine::{open_workbook_auto, Reader as WorkbookReader};
use quick_xml::events::Event;
use quick_xml::Reader as XmlReader;
use serde::Serialize;
use std::collections::HashSet;
use std::io::Read as IoRead;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;
use zip::ZipArchive;

/// Plain text and source/config files: read as-is, no parsing needed.
const PLAIN_TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "csv", "log", "rs", "py", "js", "jsx", "mjs", "cjs", "ts", "tsx", "json", "yaml",
    "yml", "toml", "xml", "ini", "java", "kt", "c", "h", "cpp", "hpp", "cc", "cs", "go", "rb",
    "php", "sh", "bash", "zsh", "css", "scss", "sass", "sql", "swift", "lua",
];
const HTML_EXTENSIONS: &[&str] = &["html", "htm"];
const RTF_EXTENSIONS: &[&str] = &["rtf"];
const PDF_EXTENSIONS: &[&str] = &["pdf"];
const XLSX_EXTENSIONS: &[&str] = &["xlsx"];
const DOCX_EXTENSIONS: &[&str] = &["docx"];
const PPTX_EXTENSIONS: &[&str] = &["pptx"];

const SKIPPED_DIR_NAMES: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".cache",
    "__pycache__",
    ".venv",
    // OS-internal folders: near-certain to appear directly under a home directory and
    // never contain user documents worth indexing, so skip them to keep a "search my
    // whole home folder" scan fast rather than crawling caches/app config for nothing.
    "Library",
    "AppData",
];

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexableFile {
    pub path: String,
    pub file_name: String,
    pub extension: String,
    pub size_bytes: u64,
    pub modified_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtractedDocument {
    pub path: String,
    pub text: String,
}

fn extension_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_lowercase())
}

pub fn is_supported(path: &Path) -> bool {
    extension_lower(path)
        .map(|ext| {
            let ext = ext.as_str();
            PLAIN_TEXT_EXTENSIONS.contains(&ext)
                || HTML_EXTENSIONS.contains(&ext)
                || RTF_EXTENSIONS.contains(&ext)
                || PDF_EXTENSIONS.contains(&ext)
                || XLSX_EXTENSIONS.contains(&ext)
                || DOCX_EXTENSIONS.contains(&ext)
                || PPTX_EXTENSIONS.contains(&ext)
        })
        .unwrap_or(false)
}

fn is_hidden_or_noisy(entry_name: &str) -> bool {
    entry_name.starts_with('.') || SKIPPED_DIR_NAMES.contains(&entry_name)
}

/// Recursively walks `root`, returning metadata for every file with a supported extension.
/// Hidden directories and common build/dependency folders are skipped for performance.
pub fn scan_directory(root: &str) -> Vec<IndexableFile> {
    let mut results = Vec::new();

    let walker = WalkDir::new(root).into_iter().filter_entry(|entry| {
        if entry.depth() == 0 {
            return true;
        }
        let name = entry.file_name().to_string_lossy();
        if entry.file_type().is_dir() {
            !is_hidden_or_noisy(&name)
        } else {
            true
        }
    });

    for entry in walker.filter_map(Result::ok) {
        if !entry.file_type().is_file() || !is_supported(entry.path()) {
            continue;
        }

        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified_ms = metadata
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        results.push(IndexableFile {
            path: entry.path().to_string_lossy().to_string(),
            file_name: entry.file_name().to_string_lossy().to_string(),
            extension: extension_lower(entry.path()).unwrap_or_default(),
            size_bytes: metadata.len(),
            modified_ms,
        });
    }

    results
}

/// Scans multiple root directories and de-duplicates files reachable from more than one root.
pub fn scan_directories(directories: &[String]) -> Vec<IndexableFile> {
    let mut seen = HashSet::new();
    let mut files = Vec::new();

    for dir in directories {
        for file in scan_directory(dir) {
            if seen.insert(file.path.clone()) {
                files.push(file);
            }
        }
    }

    files
}

fn extract_plain_text(path: &Path) -> Result<String, String> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(String::from_utf8_lossy(&bytes).into_owned()),
        Err(e) => Err(format!("failed to read {}: {e}", path.display())),
    }
}

/// Extracts a PDF's text page-by-page (rather than pdf-extract's own whole-document
/// `extract_text`), catching panics per page. pdf-extract panics - rather than
/// returning `Err` - on a number of malformed-but-common real-world constructs (e.g.
/// certain embedded Type3 fonts with missing width entries, unexpected encodings).
/// Isolating each page means one bad page/font only loses that page's text instead of
/// silently losing the entire document; an outer catch_unwind is still a safety net in
/// case loading the document itself panics.
fn extract_pdf_text(path: &Path) -> Result<String, String> {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        extract_pdf_text_page_by_page(path)
    }));

    match outcome {
        Ok(result) => result,
        Err(_) => Err(format!("PDF parser crashed while reading {}", path.display())),
    }
}

fn extract_pdf_text_page_by_page(path: &Path) -> Result<String, String> {
    let mut doc = pdf_extract::Document::load(path)
        .map_err(|e| format!("failed to open PDF {}: {e}", path.display()))?;

    if doc.is_encrypted() {
        // Best-effort, matching pdf-extract's own `extract_text`: many "encrypted" PDFs
        // in the wild just have an empty owner password and open fine with one.
        let _ = doc.decrypt("");
    }

    let mut page_numbers: Vec<u32> = doc.get_pages().keys().copied().collect();
    page_numbers.sort_unstable();

    let mut combined = String::new();
    for page_num in page_numbers {
        let page_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut text = String::new();
            let outcome = {
                let mut output = pdf_extract::PlainTextOutput::new(&mut text);
                pdf_extract::output_doc_page(&doc, &mut output, page_num)
            };
            outcome.map(|_| text)
        }));

        // A page that errors or panics is skipped - the rest of the document's text is
        // still worth keeping rather than discarding everything over one bad page.
        if let Ok(Ok(text)) = page_result {
            combined.push_str(&text);
            combined.push('\n');
        }
    }

    Ok(combined)
}

fn extract_html_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("failed to read {}: {e}", path.display()))?;
    html2text::from_read(bytes.as_slice(), 120)
        .map_err(|e| format!("failed to extract HTML text from {}: {e}", path.display()))
}

fn extract_rtf_text(path: &Path) -> Result<String, String> {
    let path_str = path
        .to_str()
        .ok_or_else(|| format!("non-UTF-8 path: {}", path.display()))?;
    rtf_parser::RtfDocument::from_filepath(path_str)
        .map(|doc| doc.get_text())
        .map_err(|e| format!("failed to parse RTF {}: {e}", path.display()))
}

/// Pulls the text content out of every `<local_tag_name>` element in `xml`, ignoring
/// namespace prefixes (so `w:t`, `a:t`, etc. all match on their local name alone).
fn extract_xml_text_nodes(xml: &str, local_tag_name: &[u8]) -> String {
    // No `trim_text`: entity references split a text run into multiple Text events
    // (see the GeneralRef handling below), and trimming each fragment independently
    // would eat the whitespace surrounding the entity from both sides.
    let mut reader = XmlReader::from_str(xml);
    let mut buf = Vec::new();
    let mut inside = false;
    let mut out = String::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == local_tag_name => inside = true,
            Ok(Event::End(e)) if e.local_name().as_ref() == local_tag_name => {
                inside = false;
                out.push(' ');
            }
            Ok(Event::Text(t)) if inside => {
                if let Ok(decoded) = t.decode() {
                    if let Ok(unescaped) = quick_xml::escape::unescape(&decoded) {
                        out.push_str(&unescaped);
                    }
                }
            }
            // Named/numeric entity references (e.g. `&amp;`, `&#38;`) arrive as their own
            // event rather than being inlined into the surrounding Text event's bytes.
            Ok(Event::GeneralRef(r)) if inside => {
                if let Ok(Some(ch)) = r.resolve_char_ref() {
                    out.push(ch);
                } else if let Ok(name) = r.decode() {
                    if let Ok(unescaped) = quick_xml::escape::unescape(&format!("&{name};")) {
                        out.push_str(&unescaped);
                    }
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    out
}

fn extract_xlsx_text(path: &Path) -> Result<String, String> {
    let mut workbook = open_workbook_auto(path)
        .map_err(|e| format!("failed to open spreadsheet {}: {e}", path.display()))?;

    let mut text = String::new();
    for sheet_name in workbook.sheet_names() {
        let Ok(range) = workbook.worksheet_range(&sheet_name) else {
            continue;
        };
        for row in range.rows() {
            let cells: Vec<String> = row
                .iter()
                .map(|cell| cell.to_string())
                .filter(|s| !s.is_empty())
                .collect();
            if !cells.is_empty() {
                text.push_str(&cells.join(" "));
                text.push('\n');
            }
        }
    }
    Ok(text)
}

fn extract_docx_text(path: &Path) -> Result<String, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("failed to read {} as a zip archive: {e}", path.display()))?;

    let mut entry = archive
        .by_name("word/document.xml")
        .map_err(|e| format!("missing word/document.xml in {}: {e}", path.display()))?;
    let mut xml = String::new();
    entry
        .read_to_string(&mut xml)
        .map_err(|e| format!("failed to read document.xml in {}: {e}", path.display()))?;

    Ok(extract_xml_text_nodes(&xml, b"t"))
}

fn pptx_slide_number(name: &str) -> u32 {
    name.trim_start_matches("ppt/slides/slide")
        .trim_end_matches(".xml")
        .parse()
        .unwrap_or(0)
}

fn extract_pptx_text(path: &Path) -> Result<String, String> {
    let file =
        std::fs::File::open(path).map_err(|e| format!("failed to open {}: {e}", path.display()))?;
    let mut archive = ZipArchive::new(file)
        .map_err(|e| format!("failed to read {} as a zip archive: {e}", path.display()))?;

    let mut slide_names: Vec<String> = archive
        .file_names()
        .filter(|name| name.starts_with("ppt/slides/slide") && name.ends_with(".xml"))
        .map(str::to_string)
        .collect();
    slide_names.sort_by_key(|name| pptx_slide_number(name));

    let mut text = String::new();
    for name in &slide_names {
        let mut xml = String::new();
        if let Ok(mut entry) = archive.by_name(name) {
            if entry.read_to_string(&mut xml).is_ok() {
                text.push_str(&extract_xml_text_nodes(&xml, b"t"));
                text.push('\n');
            }
        }
    }
    Ok(text)
}

/// Extracts raw text content from a single file, dispatching on its extension.
pub fn extract_document_text(path: &str) -> Result<ExtractedDocument, String> {
    let path_ref = Path::new(path);
    let extension = extension_lower(path_ref).unwrap_or_default();
    let ext = extension.as_str();

    let text = if PLAIN_TEXT_EXTENSIONS.contains(&ext) {
        extract_plain_text(path_ref)?
    } else if HTML_EXTENSIONS.contains(&ext) {
        extract_html_text(path_ref)?
    } else if RTF_EXTENSIONS.contains(&ext) {
        extract_rtf_text(path_ref)?
    } else if PDF_EXTENSIONS.contains(&ext) {
        extract_pdf_text(path_ref)?
    } else if XLSX_EXTENSIONS.contains(&ext) {
        extract_xlsx_text(path_ref)?
    } else if DOCX_EXTENSIONS.contains(&ext) {
        extract_docx_text(path_ref)?
    } else if PPTX_EXTENSIONS.contains(&ext) {
        extract_pptx_text(path_ref)?
    } else {
        return Err(format!("unsupported file extension: {ext}"));
    };

    Ok(ExtractedDocument {
        path: path.to_string(),
        text,
    })
}

#[tauri::command]
pub fn scan_directories_cmd(directories: Vec<String>) -> Vec<IndexableFile> {
    scan_directories(&directories)
}

#[tauri::command]
pub fn extract_document_text_cmd(path: String) -> Result<ExtractedDocument, String> {
    extract_document_text(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn scans_supported_files_and_skips_noise() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        fs::write(root.join("notes.txt"), "hello world").unwrap();
        fs::write(root.join("readme.md"), "# Title").unwrap();
        fs::write(root.join("image.png"), "not text").unwrap();

        let noisy = root.join("node_modules");
        fs::create_dir(&noisy).unwrap();
        fs::write(noisy.join("skip.txt"), "should be skipped").unwrap();

        let nested = root.join("subdir");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("nested.txt"), "nested content").unwrap();

        let files = scan_directory(root.to_str().unwrap());
        let names: HashSet<_> = files.iter().map(|f| f.file_name.clone()).collect();

        assert!(names.contains("notes.txt"));
        assert!(names.contains("readme.md"));
        assert!(names.contains("nested.txt"));
        assert!(!names.contains("image.png"));
        assert!(!names.contains("skip.txt"));
    }

    #[test]
    fn skips_os_internal_folders_under_a_home_directory() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        let library = root.join("Library");
        fs::create_dir(&library).unwrap();
        fs::write(library.join("cache.txt"), "should be skipped").unwrap();

        let app_data = root.join("AppData");
        fs::create_dir(&app_data).unwrap();
        fs::write(app_data.join("config.txt"), "should be skipped").unwrap();

        fs::write(root.join("resume.txt"), "kept").unwrap();

        let files = scan_directory(root.to_str().unwrap());
        let names: HashSet<_> = files.iter().map(|f| f.file_name.clone()).collect();

        assert!(names.contains("resume.txt"));
        assert!(!names.contains("cache.txt"));
        assert!(!names.contains("config.txt"));
    }

    #[test]
    fn deduplicates_across_overlapping_roots() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.txt"), "content").unwrap();
        let nested = root.join("nested");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("b.txt"), "content").unwrap();

        let dirs = vec![
            root.to_str().unwrap().to_string(),
            nested.to_str().unwrap().to_string(),
        ];
        let files = scan_directories(&dirs);
        let count = files.iter().filter(|f| f.file_name == "b.txt").count();
        assert_eq!(count, 1);
    }

    #[test]
    fn extracts_plain_text_files() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.txt");
        fs::write(&file_path, "some content here").unwrap();

        let doc = extract_document_text(file_path.to_str().unwrap()).unwrap();
        assert_eq!(doc.text, "some content here");
    }

    #[test]
    fn extracts_html_text() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("page.html");
        fs::write(&file_path, "<html><body><p>Hello World</p></body></html>").unwrap();

        let doc = extract_document_text(file_path.to_str().unwrap()).unwrap();
        assert!(doc.text.contains("Hello World"));
    }

    #[test]
    fn extracts_rtf_text() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.rtf");
        fs::write(&file_path, r"{\rtf1\ansi Hello World}").unwrap();

        let doc = extract_document_text(file_path.to_str().unwrap()).unwrap();
        assert!(doc.text.contains("Hello World"));
    }

    #[test]
    fn recognizes_code_and_config_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("data.csv"), "a,b,c").unwrap();
        fs::write(root.join("config.yaml"), "key: value").unwrap();

        let files = scan_directory(root.to_str().unwrap());
        let names: HashSet<_> = files.iter().map(|f| f.file_name.clone()).collect();

        assert!(names.contains("main.rs"));
        assert!(names.contains("data.csv"));
        assert!(names.contains("config.yaml"));
    }

    #[test]
    fn extract_pdf_text_returns_err_on_garbage_input_instead_of_crashing() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("not-a-real.pdf");
        fs::write(&file_path, b"this is not a valid pdf file at all").unwrap();

        let result = extract_document_text(file_path.to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn catch_unwind_converts_a_panic_into_an_err_without_crashing_the_process() {
        // Exercises the same catch_unwind + map_err pattern extract_pdf_text uses, with a
        // synthetic panic standing in for pdf-extract's internal panic on malformed PDFs -
        // confirming the wrapping mechanism itself works in this test environment.
        let result: Result<i32, String> = std::panic::catch_unwind(|| -> i32 {
            panic!("simulated third-party crate panic");
        })
        .map_err(|_| "caught".to_string());

        assert_eq!(result, Err("caught".to_string()));
    }

    #[test]
    fn rejects_unsupported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("image.png");
        fs::write(&file_path, "binary").unwrap();

        let result = extract_document_text(file_path.to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn xml_text_node_extraction_unescapes_entities() {
        let xml = r#"<w:p><w:r><w:t>Tom &amp; Jerry</w:t></w:r></w:p>"#;
        let text = extract_xml_text_nodes(xml, b"t");
        assert_eq!(text.trim(), "Tom & Jerry");
    }

    #[test]
    fn xml_text_node_extraction_ignores_other_tags() {
        let xml = r#"<w:p><w:pPr/><w:r><w:t>kept</w:t></w:r><w:other>dropped</w:other></w:p>"#;
        let text = extract_xml_text_nodes(xml, b"t");
        assert!(text.contains("kept"));
        assert!(!text.contains("dropped"));
    }

    /// Writes a minimal, uncompressed zip archive with the given `(name, contents)` entries.
    fn write_zip_fixture(path: &std::path::Path, entries: &[(&str, &str)]) {
        use std::io::Write;

        let file = fs::File::create(path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let options =
            zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, contents) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(contents.as_bytes()).unwrap();
        }
        writer.finish().unwrap();
    }

    #[test]
    fn extracts_docx_text() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("note.docx");
        let document_xml = r#"<?xml version="1.0"?>
            <w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
              <w:body>
                <w:p><w:r><w:t>Hello</w:t></w:r><w:r><w:t>World</w:t></w:r></w:p>
              </w:body>
            </w:document>"#;
        write_zip_fixture(&file_path, &[("word/document.xml", document_xml)]);

        let doc = extract_document_text(file_path.to_str().unwrap()).unwrap();
        assert!(doc.text.contains("Hello"));
        assert!(doc.text.contains("World"));
    }

    #[test]
    fn extracts_pptx_text_in_slide_order() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("deck.pptx");
        let slide = |body: &str| {
            format!(
                r#"<?xml version="1.0"?>
                <p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main">
                  <p:cSld><p:spTree><p:sp><p:txBody><a:p><a:r><a:t>{body}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld>
                </p:sld>"#
            )
        };
        let slide1 = slide("First slide");
        let slide2 = slide("Second slide");
        // Written out of order to verify extraction sorts by slide number, not zip order.
        write_zip_fixture(
            &file_path,
            &[
                ("ppt/slides/slide2.xml", &slide2),
                ("ppt/slides/slide1.xml", &slide1),
            ],
        );

        let doc = extract_document_text(file_path.to_str().unwrap()).unwrap();
        let first_pos = doc.text.find("First slide").unwrap();
        let second_pos = doc.text.find("Second slide").unwrap();
        assert!(first_pos < second_pos);
    }

    #[test]
    fn recognizes_office_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write_zip_fixture(&root.join("report.docx"), &[("word/document.xml", "<x/>")]);
        write_zip_fixture(&root.join("deck.pptx"), &[("ppt/slides/slide1.xml", "<x/>")]);
        fs::write(root.join("sheet.xlsx"), "not a real xlsx, just checking scan filtering").unwrap();

        let files = scan_directory(root.to_str().unwrap());
        let names: HashSet<_> = files.iter().map(|f| f.file_name.clone()).collect();

        assert!(names.contains("report.docx"));
        assert!(names.contains("deck.pptx"));
        assert!(names.contains("sheet.xlsx"));
    }
}
