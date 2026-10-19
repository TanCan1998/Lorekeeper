//! 对照上游 EPUB 合成用例；不读用户文件、不创建包注册或网络请求。

use std::io::Write;

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use super::*;

const CONTAINER: &str =
    "<container><rootfiles><rootfile full-path=\"book.opf\"/></rootfiles></container>";
const PACKAGE: &str = "<package><manifest><item id=\"chapter\" href=\"chapter.xhtml\" media-type=\"application/xhtml+xml\"/></manifest><spine><itemref idref=\"chapter\"/></spine></package>";
const DOCUMENT: &str =
    "<html><head><title>Arrival</title></head><body><p>正文内容。</p></body></html>";

fn archive(entries: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn simple_book(extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut entries = vec![
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("book.opf", PACKAGE.as_bytes()),
        ("chapter.xhtml", DOCUMENT.as_bytes()),
    ];
    entries.extend_from_slice(extra);
    archive(&entries, CompressionMethod::Stored)
}

fn extract(bytes: &[u8]) -> Result<Vec<EpubChapter>, String> {
    extract_epub_chapters(bytes, EpubLimits::default())
}

#[test]
fn epub2_follows_spine_and_preserves_unicode_test() {
    let container =
        "<container><rootfiles><rootfile full-path='OPS/content.opf'/></rootfiles></container>";
    let package = "<package><manifest>
        <item id='second' href='chapters/第二章.xhtml' media-type='application/xhtml+xml'/>
        <item id='first' href='chapters/chapter-1.xhtml' media-type='application/xhtml+xml'/>
        </manifest><spine><itemref idref='first'/><itemref idref='second'/></spine></package>";
    let first = "<html><head><title>初遇</title></head><body><h1>第一章 初遇</h1><p>雨落在长安城。她说：“你好，世界。”</p></body></html>";
    let second = "<html><head><title>重逢</title></head><body><h1>第二章 重逢</h1><p>Café 灯火未熄，陆云飞回来了。</p></body></html>";
    let bytes = archive(
        &[
            ("META-INF/container.xml", container.as_bytes()),
            ("OPS/content.opf", package.as_bytes()),
            ("OPS/chapters/第二章.xhtml", second.as_bytes()),
            ("OPS/chapters/chapter-1.xhtml", first.as_bytes()),
        ],
        CompressionMethod::Stored,
    );
    assert_eq!(
        extract(&bytes).unwrap(),
        vec![
            EpubChapter {
                title: "第一章 初遇".to_string(),
                content: "第一章 初遇\n\n雨落在长安城。她说：“你好，世界。”".to_string(),
            },
            EpubChapter {
                title: "第二章 重逢".to_string(),
                content: "第二章 重逢\n\nCafé 灯火未熄，陆云飞回来了。".to_string(),
            },
        ]
    );
}

#[test]
fn epub3_skips_navigation_non_linear_and_non_text_resources_test() {
    let container = "<c:container><c:rootfiles><c:rootfile full-path='book/package.opf'/></c:rootfiles></c:container>";
    let package = "<opf:package><opf:manifest>
        <opf:item id='nav' href='nav.xhtml' media-type='application/xhtml+xml' properties='nav'/>
        <opf:item id='one' href='text/part%201.html?mode=read#start' media-type='text/html'/>
        <opf:item id='two' href='text/part-2.xhtml' media-type='application/xhtml+xml'/>
        <opf:item id='notes' href='notes.xhtml' media-type='application/xhtml+xml'/>
        <opf:item id='image' href='cover.png' media-type='image/png'/>
        </opf:manifest><opf:spine>
        <opf:itemref idref='nav'/><opf:itemref idref='image'/><opf:itemref idref='two'/>
        <opf:itemref idref='one'/><opf:itemref idref='notes' linear='NO'/>
        </opf:spine></opf:package>";
    // 不被消费的 nav、附注与图片无需解压或存在；顺序仅由 spine 决定。
    let bytes = archive(&[
        ("META-INF/container.xml", container.as_bytes()),
        ("book/package.opf", package.as_bytes()),
        ("book/text/part 1.html", "<html><head><title>First &amp; Last</title></head><body><p>你好&#x4E16;&#30028;。</p></body></html>".as_bytes()),
        ("book/text/part-2.xhtml", "<html><body><h2>第二幕</h2><p>élan &amp; courage</p></body></html>".as_bytes()),
    ], CompressionMethod::Deflated);
    assert_eq!(
        extract(&bytes).unwrap(),
        vec![
            EpubChapter {
                title: "第二幕".to_string(),
                content: "第二幕\n\nélan & courage".to_string()
            },
            EpubChapter {
                title: "First & Last".to_string(),
                content: "你好世界。".to_string()
            },
        ]
    );
}

#[test]
fn utf16_entries_support_both_endiannesses_with_and_without_bom_test() {
    for little in [true, false] {
        for bom in [true, false] {
            let encode = |source: &str| {
                let mut bytes = Vec::new();
                if bom {
                    bytes.extend_from_slice(if little { &[0xff, 0xfe] } else { &[0xfe, 0xff] });
                }
                for unit in
                    format!("<?xml version='1.0' encoding='UTF-16'?>{source}").encode_utf16()
                {
                    bytes.extend_from_slice(&if little {
                        unit.to_le_bytes()
                    } else {
                        unit.to_be_bytes()
                    });
                }
                bytes
            };
            let container = encode(CONTAINER);
            let package = encode(PACKAGE);
            let document = encode(DOCUMENT);
            let bytes = archive(
                &[
                    ("META-INF/container.xml", &container),
                    ("book.opf", &package),
                    ("chapter.xhtml", &document),
                ],
                CompressionMethod::Stored,
            );
            assert_eq!(
                extract(&bytes).unwrap(),
                vec![EpubChapter {
                    title: "Arrival".to_string(),
                    content: "正文内容。".to_string(),
                }]
            );
        }
    }
}

#[test]
fn strict_decoding_rejects_invalid_utf8_utf16_and_preserves_bom_semantics_test() {
    for document in [
        &[0xff][..],
        &[0xff, 0xfe, 0x41][..],
        &[0xfe, 0xff, 0xd8, 0x00][..],
    ] {
        let bytes = archive(
            &[
                ("META-INF/container.xml", CONTAINER.as_bytes()),
                ("book.opf", PACKAGE.as_bytes()),
                ("chapter.xhtml", document),
            ],
            CompressionMethod::Stored,
        );
        assert_eq!(extract(&bytes).unwrap_err(), INVALID);
    }
    assert_eq!(decode_document(&[0xef, 0xbb, 0xbf, b'A']).unwrap(), "A");
}

#[test]
fn html_extraction_removes_executable_markup_and_keeps_paragraphs_test() {
    let document = "<html><head><title>A &amp; B</title><style>head secret</style></head><body><!-- hidden --><style>body secret</style><script>not text</script><p>甲&nbsp; 乙<br/>丙</p><div>丁&#x1F600;</div><p>&unknown; &#99999999;</p></body></html>";
    let chapter = extract_document(document, "OPS/chapter.xhtml");
    assert_eq!(chapter.title, "A & B");
    assert_eq!(
        chapter.content,
        "甲 乙\n丙\n\n丁😀\n\n&unknown; &#99999999;"
    );
    assert_eq!(
        extract_document("<body><p>正文</p></body>", "OPS/章节.xhtml").title,
        "章节"
    );
}

#[test]
fn archive_paths_and_uri_references_are_normalized_without_filesystem_access_test() {
    assert_eq!(
        normalize_archive_path("/OPS/a/../b.xhtml").unwrap(),
        "OPS/b.xhtml"
    );
    assert_eq!(
        resolve_manifest_path("OPS/text", "../%E7%AC%AC%E4%B8%80%E7%AB%A0.xhtml#x").unwrap(),
        "OPS/第一章.xhtml"
    );
    for value in [
        "",
        "../bad",
        "a/../../bad",
        "bad\\name",
        "bad\0name",
        ".",
        "/",
    ] {
        assert_eq!(normalize_archive_path(value).unwrap_err(), INVALID);
    }
    for value in ["%", "%GG", "%ff", "#fragment"] {
        assert_eq!(decode_reference(value).unwrap_err(), INVALID);
    }
    assert_eq!(
        resolve_manifest_path("OPS", "../../outside.xhtml").unwrap_err(),
        INVALID
    );
}

#[test]
fn corrupt_missing_container_and_zip_slip_are_invalid_test() {
    assert_eq!(extract(b"not a zip archive").unwrap_err(), INVALID);
    for entries in [
        vec![("book.opf", PACKAGE.as_bytes())],
        vec![("../META-INF/container.xml", CONTAINER.as_bytes())],
    ] {
        assert_eq!(
            extract(&archive(&entries, CompressionMethod::Stored)).unwrap_err(),
            INVALID
        );
    }
}

#[test]
fn duplicate_normalized_archive_names_are_rejected_test() {
    let bytes = simple_book(&[("OPS/../book.opf", PACKAGE.as_bytes())]);
    assert_eq!(extract(&bytes).unwrap_err(), INVALID);
}

#[test]
fn raw_duplicate_names_cannot_hide_entries_from_zip_index_test() {
    let mut bytes = simple_book(&[("copy.opf", PACKAGE.as_bytes())]);
    // 两个同长名称在本地头、中央目录都改为相同，模拟 zip 索引会合并的输入。
    for offset in 0..=bytes.len() - b"copy.opf".len() {
        if bytes[offset..].starts_with(b"copy.opf") {
            bytes[offset..offset + 8].copy_from_slice(b"book.opf");
        }
    }
    assert_eq!(extract(&bytes).unwrap_err(), INVALID);
    let limits = EpubLimits {
        max_entries: 3,
        ..EpubLimits::default()
    };
    assert_eq!(
        extract_epub_chapters(&bytes, limits).unwrap_err(),
        EXPANSION_LIMIT
    );
}

#[test]
fn missing_package_spine_targets_and_ambiguous_manifests_are_invalid_test() {
    for package in [
        "<package/>",
        "<package><spine><itemref idref='missing'/></spine></package>",
        "<package><manifest><item id='a' href='missing.xhtml' media-type='application/xhtml+xml'/></manifest><spine><itemref idref='a'/></spine></package>",
        "<package><manifest><item id='a' href='one.xhtml'/><item id='a' href='two.xhtml'/></manifest></package>",
        "<package><manifest><item id='a' href='one.xhtml'/><item id='b' href='./one.xhtml'/></manifest></package>",
        "<package><manifest><item id='a' href='%GG'/></manifest></package>",
        "<package><manifest><item id='a'/></manifest></package>",
    ] {
        let bytes = archive(&[
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("book.opf", package.as_bytes()),
        ], CompressionMethod::Stored);
        assert_eq!(extract(&bytes).unwrap_err(), INVALID);
    }
    let bytes = archive(
        &[("META-INF/container.xml", CONTAINER.as_bytes())],
        CompressionMethod::Stored,
    );
    assert_eq!(extract(&bytes).unwrap_err(), INVALID);
}

#[test]
fn empty_documents_do_not_create_chapters_test() {
    let bytes = archive(
        &[
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("book.opf", PACKAGE.as_bytes()),
            ("chapter.xhtml", b"<html><body><p> </p></body></html>"),
        ],
        CompressionMethod::Stored,
    );
    assert_eq!(extract(&bytes).unwrap_err(), INVALID);
}

fn encrypted_book(algorithm: &str, target: &str, font_present: bool) -> Vec<u8> {
    let encryption = format!("<encryption><enc:EncryptedData><enc:EncryptionMethod Algorithm='{algorithm}'/><enc:CipherData><enc:CipherReference URI='{target}'/></enc:CipherData></enc:EncryptedData></encryption>");
    let package = "<package><manifest>
        <item id='chapter' href='chapter.xhtml' media-type='application/xhtml+xml'/>
        <item id='font' href='font.otf' media-type='application/vnd.ms-opentype'/>
        </manifest><spine><itemref idref='chapter'/></spine></package>";
    let mut entries = vec![
        ("META-INF/container.xml", CONTAINER.as_bytes()),
        ("META-INF/encryption.xml", encryption.as_bytes()),
        ("book.opf", package.as_bytes()),
        ("chapter.xhtml", DOCUMENT.as_bytes()),
    ];
    if font_present {
        entries.push(("font.otf", &[0, 1, 2, 3]));
    }
    archive(&entries, CompressionMethod::Stored)
}

#[test]
fn standard_font_obfuscation_is_allowed_only_for_existing_manifest_fonts_test() {
    for algorithm in [
        "http://www.idpf.org/2008/embedding",
        "http://ns.adobe.com/pdf/enc#RC",
    ] {
        let bytes = encrypted_book(algorithm, "font.otf", true);
        assert_eq!(extract(&bytes).unwrap()[0].content, "正文内容。");
        let missing = encrypted_book(algorithm, "font.otf", false);
        assert_eq!(extract(&missing).unwrap_err(), DRM_UNSUPPORTED);
        let disguised = encrypted_book(algorithm, "chapter.xhtml", true);
        assert_eq!(extract(&disguised).unwrap_err(), DRM_UNSUPPORTED);
    }
}

#[test]
fn real_drm_unknown_algorithms_and_undeclared_targets_are_rejected_test() {
    for (algorithm, target) in [
        (
            "http://www.w3.org/2001/04/xmlenc#aes256-cbc",
            "chapter.xhtml",
        ),
        ("urn:example:unknown", "font.otf"),
        ("http://www.idpf.org/2008/embedding", "missing.otf"),
    ] {
        let bytes = encrypted_book(algorithm, target, true);
        assert_eq!(extract(&bytes).unwrap_err(), DRM_UNSUPPORTED);
    }
}

#[test]
fn malformed_encryption_metadata_is_invalid_not_drm_test() {
    for encryption in [
        "<encryption/>",
        "<encryption><EncryptedData/></encryption>",
        "<encryption><EncryptedData></EncryptedData></encryption>",
        "<encryption><EncryptedData><EncryptionMethod Algorithm='x'/></EncryptedData></encryption>",
        "<encryption><EncryptedData><EncryptionMethod Algorithm='x'/><CipherReference URI='%GG'/></EncryptedData></encryption>",
    ] {
        let bytes = simple_book(&[("META-INF/encryption.xml", encryption.as_bytes())]);
        assert_eq!(extract(&bytes).unwrap_err(), INVALID);
    }
}

#[test]
fn encrypted_zip_entries_are_rejected_before_body_reading_test() {
    let mut bytes = simple_book(&[]);
    // 合成加密标志，不生成密码或依赖加密 feature；本地头与中央目录同时更新。
    for offset in 0..bytes.len().saturating_sub(10) {
        let flag = if bytes[offset..].starts_with(b"PK\x03\x04") {
            Some(offset + 6)
        } else if bytes[offset..].starts_with(b"PK\x01\x02") {
            Some(offset + 8)
        } else {
            None
        };
        if let Some(flag) = flag {
            bytes[flag] |= 1;
        }
    }
    assert_eq!(extract(&bytes).unwrap_err(), DRM_UNSUPPORTED);
}

#[test]
fn entry_count_expanded_entry_and_cumulative_bytes_are_bounded_test() {
    let bytes = simple_book(&[]);
    for limits in [
        EpubLimits {
            max_entries: 2,
            ..EpubLimits::default()
        },
        EpubLimits {
            max_entry_bytes: CONTAINER.len() - 1,
            ..EpubLimits::default()
        },
        EpubLimits {
            max_extracted_bytes: CONTAINER.len() + PACKAGE.len() - 1,
            ..EpubLimits::default()
        },
    ] {
        assert_eq!(
            extract_epub_chapters(&bytes, limits).unwrap_err(),
            EXPANSION_LIMIT
        );
    }
    let exact = EpubLimits {
        max_entries: 3,
        max_entry_bytes: PACKAGE.len().max(DOCUMENT.len()).max(CONTAINER.len()),
        max_extracted_bytes: CONTAINER.len() + PACKAGE.len() + DOCUMENT.len(),
    };
    assert_eq!(extract_epub_chapters(&bytes, exact).unwrap().len(), 1);
}

#[test]
fn zip64_entries_are_read_without_a_separate_archive_parser_test() {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, content) in [
        ("META-INF/container.xml", CONTAINER),
        ("book.opf", PACKAGE),
        ("chapter.xhtml", DOCUMENT),
    ] {
        let options = SimpleFileOptions::default()
            .compression_method(CompressionMethod::Stored)
            .large_file(true);
        writer.start_file(name, options).unwrap();
        writer.write_all(content.as_bytes()).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    assert_eq!(extract(&bytes).unwrap()[0].content, "正文内容。");
}

#[test]
fn forged_declared_entry_size_cannot_bypass_limits_or_succeed_test() {
    let original = simple_book(&[]);
    let archive = ZipArchive::new(Cursor::new(original.as_slice())).unwrap();
    let directory_start = archive.central_directory_start() as usize;
    for (declared_size, expected) in [
        (1u32, INVALID),
        (EPUB_MAX_ENTRY_BYTES as u32 + 1, EXPANSION_LIMIT),
    ] {
        let mut bytes = original.clone();
        // 第一个中央目录条目的未压缩大小；其实际数据与 CRC 均不变。
        bytes[directory_start + 24..directory_start + 28]
            .copy_from_slice(&declared_size.to_le_bytes());
        assert_eq!(extract(&bytes).unwrap_err(), expected);
    }
}

#[test]
fn repeated_spine_documents_still_consume_expansion_budget_test() {
    let package = PACKAGE.replace("</spine>", "<itemref idref='chapter'/></spine>");
    let bytes = archive(
        &[
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("book.opf", package.as_bytes()),
            ("chapter.xhtml", DOCUMENT.as_bytes()),
        ],
        CompressionMethod::Stored,
    );
    let limits = EpubLimits {
        max_extracted_bytes: CONTAINER.len() + package.len() + DOCUMENT.len(),
        ..EpubLimits::default()
    };
    assert_eq!(
        extract_epub_chapters(&bytes, limits).unwrap_err(),
        EXPANSION_LIMIT
    );
}

#[test]
fn crc_corruption_is_rejected_instead_of_returning_changed_author_text_test() {
    let mut bytes = simple_book(&[]);
    let offset = bytes
        .windows(DOCUMENT.len())
        .position(|window| window == DOCUMENT.as_bytes())
        .unwrap();
    bytes[offset + 1] ^= 1;
    assert_eq!(extract(&bytes).unwrap_err(), INVALID);
}
