//! Offline tests for `extract` and `archive`.
//!
//! Every archive here is built in a temp directory and read back by the same
//! code, so the suite needs no network and no `tar`/`unzip` on `PATH`. The
//! hand-written traversal archives at the bottom are the point of the file:
//! they are what a malicious download looks like.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use chorefile::builtins::pack::{self, Codec, Format, Target};
use chorefile::exec::{Ctx, EnvOverlay, Output};

// ---------------------------------------------------------------------------
// harness
// ---------------------------------------------------------------------------

/// Run a builtin the way the interpreter would, and hand back its exit status
/// plus whatever it printed.
fn run(dir: &Path, dry: bool, argv: &[&str]) -> chorefile::Result<(Output, String)> {
    let args: Vec<String> = argv.iter().map(|s| s.to_string()).collect();
    let builtin = pack::lookup(&args[0]).expect("builtin");
    let mut out: Vec<u8> = Vec::new();
    let mut err: Vec<u8> = Vec::new();
    let result = {
        let mut ctx = Ctx {
            args: &args,
            cwd: dir,
            root: dir,
            task: "",
            stdin: None,
            env: &EnvOverlay::default(),
            dry,
            force: false,
            out: &mut out,
            err: &mut err,
            interactive: false,
        };
        builtin(&mut ctx)
    };
    result.map(|o| (o, String::from_utf8_lossy(&out).into_owned()))
}

fn ok(dir: &Path, argv: &[&str]) {
    let (output, _) = run(dir, false, argv).unwrap_or_else(|e| panic!("{argv:?}: {e}"));
    assert!(output.success(), "{argv:?} exited {}", output.code);
}

fn err(dir: &Path, argv: &[&str]) -> String {
    match run(dir, false, argv) {
        Ok((o, _)) => panic!("{argv:?} unexpectedly succeeded with code {}", o.code),
        Err(e) => e.to_string(),
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A small tree: `src/`, one nested file, one executable.
fn sample_tree(root: &Path) -> PathBuf {
    let src = root.join("src");
    write(&src.join("README.md"), "hello\n");
    write(&src.join("bin/tool"), "#!/bin/sh\necho hi\n");
    write(&src.join("deep/nested/leaf.txt"), "leaf\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(src.join("bin/tool"), fs::Permissions::from_mode(0o755)).unwrap();
    }
    src
}

// ---------------------------------------------------------------------------
// round trips
// ---------------------------------------------------------------------------

/// Every supported container survives archive → extract with its contents and
/// its layout intact.
#[test]
fn every_format_round_trips() {
    for name in [
        "out.zip",
        "out.tar",
        "out.tar.gz",
        "out.tar.xz",
        "out.tar.zst",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sample_tree(dir);

        ok(dir, &["archive", "src", name]);
        assert!(dir.join(name).is_file(), "{name} was not created");

        ok(dir, &["extract", name, "unpacked"]);
        let root = dir.join("unpacked/src");
        assert_eq!(read(&root.join("README.md")), "hello\n", "{name}");
        assert_eq!(read(&root.join("deep/nested/leaf.txt")), "leaf\n", "{name}");
    }
}

#[cfg(unix)]
#[test]
fn the_executable_bit_survives_a_round_trip() {
    use std::os::unix::fs::PermissionsExt;
    for name in ["out.zip", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sample_tree(dir);
        ok(dir, &["archive", "src", name]);
        ok(dir, &["extract", name, "unpacked"]);

        let tool = dir.join("unpacked/src/bin/tool");
        let mode = fs::metadata(&tool).unwrap().permissions().mode();
        assert!(
            mode & 0o111 != 0,
            "{name}: lost the executable bit ({mode:o})"
        );

        let plain = dir.join("unpacked/src/README.md");
        let mode = fs::metadata(&plain).unwrap().permissions().mode();
        assert!(
            mode & 0o111 == 0,
            "{name}: invented an executable bit ({mode:o})"
        );
    }
}

/// A bare `.gz` holds one file, not a tar, and `extract` must notice.
#[test]
fn a_bare_compressed_file_extracts_to_one_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let gz = dir.join("notes.txt.gz");
    let mut enc = flate2::write::GzEncoder::new(
        fs::File::create(&gz).unwrap(),
        flate2::Compression::default(),
    );
    enc.write_all(b"just text\n").unwrap();
    enc.finish().unwrap();

    ok(dir, &["extract", "notes.txt.gz", "notes.txt"]);
    assert_eq!(read(&dir.join("notes.txt")), "just text\n");

    // A destination that looks like a directory keeps the inner name.
    ok(dir, &["extract", "notes.txt.gz", "into/"]);
    assert_eq!(read(&dir.join("into/notes.txt")), "just text\n");
}

/// An archive whose name says nothing still extracts, from its magic bytes.
#[test]
fn the_format_falls_back_to_a_content_sniff() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    sample_tree(dir);
    ok(dir, &["archive", "src", "out.zip"]);
    fs::rename(dir.join("out.zip"), dir.join("mystery")).unwrap();

    ok(dir, &["extract", "mystery", "unpacked"]);
    assert_eq!(read(&dir.join("unpacked/src/README.md")), "hello\n");
}

// ---------------------------------------------------------------------------
// what lands at the top level
// ---------------------------------------------------------------------------

/// The shape a staged native package has: two directories a consumer wants
/// unpacked straight into its own tree.
fn pkg_tree(root: &Path) {
    write(&root.join("pkg/lib/libggml.a"), "lib\n");
    write(&root.join("pkg/include/ggml.h"), "hdr\n");
}

/// Two separate trees, each of which should keep its own name.
fn bundle_tree(root: &Path) {
    write(&root.join("sona/bin/sona"), "sona\n");
    write(&root.join("ffmpeg/bin/ffmpeg"), "ffmpeg\n");
}

/// Every entry name in an archive, sorted, with the trailing slash that marks
/// a directory trimmed off so zip and tar compare against the same list.
fn names(path: &Path) -> Vec<String> {
    use std::io::Read;

    let file = fs::File::open(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let name = path.to_string_lossy().into_owned();
    let mut out: Vec<String> = Vec::new();

    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(file).unwrap();
        for i in 0..zip.len() {
            out.push(zip.by_index(i).unwrap().name().to_string());
        }
    } else {
        let reader: Box<dyn Read> = if name.ends_with(".gz") {
            Box::new(flate2::read::GzDecoder::new(file))
        } else {
            Box::new(file)
        };
        for entry in tar::Archive::new(reader).entries().unwrap() {
            let entry = entry.unwrap();
            out.push(entry.path().unwrap().to_string_lossy().into_owned());
        }
    }

    for entry in &mut out {
        while entry.ends_with('/') {
            entry.pop();
        }
    }
    out.sort();
    out
}

/// The documented form: the source's own name is the one top-level entry.
#[test]
fn a_plain_source_keeps_its_own_name_on_top() {
    for archive in ["out.zip", "out.tar", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        pkg_tree(dir);

        ok(dir, &["archive", "pkg", archive]);
        assert_eq!(
            names(&dir.join(archive)),
            [
                "pkg",
                "pkg/include",
                "pkg/include/ggml.h",
                "pkg/lib",
                "pkg/lib/libggml.a",
            ],
            "{archive}"
        );
    }
}

/// A trailing slash means the contents, so nothing gains a directory level.
#[test]
fn a_trailing_slash_packs_the_contents_at_the_root() {
    for archive in ["out.zip", "out.tar", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        pkg_tree(dir);

        ok(dir, &["archive", "pkg/", archive]);
        assert_eq!(
            names(&dir.join(archive)),
            ["include", "include/ggml.h", "lib", "lib/libggml.a"],
            "{archive}"
        );
    }
}

/// Several sources become several top-level entries in one archive.
#[test]
fn several_sources_pack_side_by_side() {
    for archive in ["out.zip", "out.tar", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        bundle_tree(dir);

        ok(dir, &["archive", "sona", "ffmpeg", archive]);
        assert_eq!(
            names(&dir.join(archive)),
            [
                "ffmpeg",
                "ffmpeg/bin",
                "ffmpeg/bin/ffmpeg",
                "sona",
                "sona/bin",
                "sona/bin/sona",
            ],
            "{archive}"
        );
    }
}

/// The two spellings compose: contents of one directory beside a loose file.
#[test]
fn contents_and_named_sources_mix() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    pkg_tree(dir);
    write(&dir.join("LICENSE"), "MIT\n");

    ok(dir, &["archive", "pkg/", "LICENSE", "out.tar"]);
    assert_eq!(
        names(&dir.join("out.tar")),
        [
            "LICENSE",
            "include",
            "include/ggml.h",
            "lib",
            "lib/libggml.a",
        ]
    );
}

/// Reproducibility is not lost by the new forms: same tree, same bytes.
#[test]
fn the_new_forms_are_reproducible() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    pkg_tree(dir);
    bundle_tree(dir);

    ok(dir, &["archive", "pkg/", "sona", "one.tar.gz"]);
    ok(dir, &["archive", "pkg/", "sona", "two.tar.gz"]);
    assert_eq!(
        fs::read(dir.join("one.tar.gz")).unwrap(),
        fs::read(dir.join("two.tar.gz")).unwrap(),
        "a flat multi-source archive is not reproducible"
    );
}

/// The whole point of the flat form: the consumer unpacks straight into its
/// own directory and finds `lib/` and `include/` there, not `pkg/lib`.
#[test]
fn a_flat_archive_round_trips_into_a_bare_directory() {
    for archive in ["out.zip", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        pkg_tree(dir);

        ok(dir, &["archive", "pkg/", archive]);
        ok(dir, &["extract", archive, "third_party"]);
        assert_eq!(read(&dir.join("third_party/lib/libggml.a")), "lib\n");
        assert_eq!(read(&dir.join("third_party/include/ggml.h")), "hdr\n");
        assert!(!dir.join("third_party/pkg").exists(), "{archive}");
    }
}

/// Two sources with the same base name would collide, and a silent overwrite
/// on extraction is worse than a refusal.
#[test]
fn colliding_sources_are_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write(&dir.join("debug/lib/a.a"), "a\n");
    write(&dir.join("release/lib/b.a"), "b\n");

    let message = err(dir, &["archive", "debug/lib", "release/lib", "out.tar"]);
    assert!(message.contains("lib"), "{message}");
    assert!(!dir.join("out.tar").exists());
}

/// A trailing slash on something that is not a directory is a mistake worth
/// naming, not a file quietly packed under its own name.
#[test]
fn a_trailing_slash_on_a_file_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    write(&dir.join("notes.txt"), "hi\n");

    let message = err(dir, &["archive", "notes.txt/", "out.tar"]);
    assert!(message.contains("notes.txt"), "{message}");
    assert!(message.contains("not a directory"), "{message}");
}

/// Getting the order wrong cannot silently pack the destination: the last
/// argument must name a supported container, and every source must exist.
#[test]
fn a_misordered_multi_source_call_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    bundle_tree(dir);

    // Destination first: `ffmpeg` is read as the destination and has no
    // archive extension, and the message says which argument that was.
    let message = err(dir, &["archive", "out.tar.gz", "sona", "ffmpeg"]);
    assert!(message.contains("ffmpeg"), "{message}");
    assert!(message.contains("last argument"), "{message}");

    // A source that does not exist is named rather than skipped.
    let message = err(dir, &["archive", "sona", "absent", "out.tar.gz"]);
    assert!(message.contains("absent"), "{message}");
    assert!(!dir.join("out.tar.gz").exists());

    // A lone argument is still a usage error.
    let message = err(dir, &["archive", "out.tar.gz"]);
    assert!(message.contains("<src...> <dest>"), "{message}");
}

// ---------------------------------------------------------------------------
// --strip and --member
// ---------------------------------------------------------------------------

#[test]
fn strip_drops_leading_components() {
    for name in ["out.zip", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sample_tree(dir);
        ok(dir, &["archive", "src", name]);

        // One level removes the `src/` the archive was built with.
        ok(dir, &["extract", name, "flat", "--strip", "1"]);
        assert_eq!(read(&dir.join("flat/README.md")), "hello\n", "{name}");
        assert!(!dir.join("flat/src").exists(), "{name}");

        // Two levels drop `src/README.md` entirely and keep only what is
        // deeper, rather than erroring on the entries that vanish.
        ok(dir, &["extract", name, "deeper", "--strip", "2"]);
        assert!(!dir.join("deeper/README.md").exists(), "{name}");
        assert_eq!(
            read(&dir.join("deeper/nested/leaf.txt")),
            "leaf\n",
            "{name}"
        );
    }
}

#[test]
fn member_extracts_exactly_one_entry() {
    for name in ["out.zip", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sample_tree(dir);
        ok(dir, &["archive", "src", name]);

        // By full path in the archive.
        ok(dir, &["extract", name, "one", "--member", "src/README.md"]);
        assert_eq!(read(&dir.join("one/src/README.md")), "hello\n", "{name}");
        assert!(!dir.join("one/src/bin/tool").exists(), "{name}");

        // By filename alone, combined with --strip.
        ok(
            dir,
            &["extract", name, "two", "--member", "tool", "--strip", "1"],
        );
        assert!(dir.join("two/bin/tool").is_file(), "{name}");
        assert!(!dir.join("two/README.md").exists(), "{name}");
    }
}

#[test]
fn a_missing_member_is_an_error_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    sample_tree(dir);
    ok(dir, &["archive", "src", "out.tar"]);

    let message = err(dir, &["extract", "out.tar", "one", "--member", "absent"]);
    assert!(message.contains("absent"), "{message}");
}

// ---------------------------------------------------------------------------
// --flatten
// ---------------------------------------------------------------------------

/// Every path under `dir`, relative, sorted, `/`-separated, with a trailing
/// `/` on directories, so a whole extraction can be asserted in one go.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, at: &Path, out: &mut Vec<String>) {
        for entry in fs::read_dir(at).unwrap_or_else(|e| panic!("{}: {e}", at.display())) {
            let path = entry.unwrap().path();
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            // `symlink_metadata`, so a symlink to a directory is a leaf here
            // rather than something to descend into.
            let is_dir = fs::symlink_metadata(&path).unwrap().is_dir();
            out.push(if is_dir { format!("{rel}/") } else { rel });
            if is_dir {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The motivating case: one binary out of a nested archive, landing at a path
/// the chorefile picked rather than one the archive did.
#[test]
fn flatten_with_member_lands_one_file_directly_in_dest() {
    for name in ["out.zip", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sample_tree(dir);
        ok(dir, &["archive", "src", name]);

        // Without the flag, the entry keeps the path it had in the archive.
        ok(dir, &["extract", name, "kept", "--member", "tool"]);
        assert_eq!(
            tree(&dir.join("kept")),
            ["src/", "src/bin/", "src/bin/tool"],
            "{name}"
        );

        // With it, exactly one file, named after the member and nothing else.
        ok(
            dir,
            &["extract", name, "got", "--member", "tool", "--flatten"],
        );
        assert_eq!(tree(&dir.join("got")), ["tool"], "{name}");
        assert_eq!(
            read(&dir.join("got/tool")),
            "#!/bin/sh\necho hi\n",
            "{name}"
        );
    }
}

/// The mode still rides along, which is the whole reason a chorefile pulls a
/// binary out of an archive in the first place.
#[cfg(unix)]
#[test]
fn a_flattened_member_keeps_its_executable_bit() {
    use std::os::unix::fs::PermissionsExt;
    for name in ["out.zip", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sample_tree(dir);
        ok(dir, &["archive", "src", name]);
        ok(
            dir,
            &["extract", name, "bin", "--member", "tool", "--flatten"],
        );

        let mode = fs::metadata(dir.join("bin/tool"))
            .unwrap()
            .permissions()
            .mode();
        assert!(
            mode & 0o111 != 0,
            "{name}: lost the executable bit ({mode:o})"
        );
    }
}

/// Without `--member`, a whole nested archive collapses to its leaves: the
/// files land loose in `dest` and not one directory is created.
#[test]
fn flatten_collapses_a_whole_archive_to_its_files() {
    for name in ["out.zip", "out.tar", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        sample_tree(dir);
        ok(dir, &["archive", "src", name]);

        ok(dir, &["extract", name, "flat", "--flatten"]);
        assert_eq!(
            tree(&dir.join("flat")),
            ["README.md", "leaf.txt", "tool"],
            "{name}"
        );
        assert_eq!(read(&dir.join("flat/leaf.txt")), "leaf\n", "{name}");
    }
}

/// Two entries with the same base name are the one thing flattening can lose,
/// so they are refused by name rather than silently resolved by archive order.
#[test]
fn colliding_flattened_entries_are_refused() {
    for name in ["out.zip", "out.tar.gz"] {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        write(&dir.join("pkg/debug/tool"), "debug\n");
        write(&dir.join("pkg/release/tool"), "release\n");
        ok(dir, &["archive", "pkg/", name]);

        let message = err(dir, &["extract", name, "flat", "--flatten"]);
        assert!(message.contains("debug/tool"), "{name}: {message}");
        assert!(message.contains("release/tool"), "{name}: {message}");
        assert!(message.contains("--member"), "{name}: {message}");

        // A `--member` narrow enough to pick one of them still works, which is
        // the fix the message points at.
        ok(
            dir,
            &[
                "extract",
                name,
                "one",
                "--member",
                "debug/tool",
                "--flatten",
            ],
        );
        assert_eq!(tree(&dir.join("one")), ["tool"], "{name}");
        assert_eq!(read(&dir.join("one/tool")), "debug\n", "{name}");
    }
}

/// Both flags drop path components, and combining them has two plausible
/// readings, so it is a usage error rather than a guess.
#[test]
fn strip_and_flatten_cannot_be_combined() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    sample_tree(dir);
    ok(dir, &["archive", "src", "out.tar"]);

    let message = err(
        dir,
        &["extract", "out.tar", "flat", "--strip", "1", "--flatten"],
    );
    assert!(message.contains("--strip"), "{message}");
    assert!(message.contains("--flatten"), "{message}");
    assert!(!dir.join("flat").exists());

    // Even `--strip 0`, which would have been a no-op: the pair is refused on
    // sight so the rule needs no footnote.
    let message = err(
        dir,
        &["extract", "out.tar", "flat", "--flatten", "--strip", "0"],
    );
    assert!(message.contains("cannot be combined"), "{message}");
}

/// A compressed single file has no entries to flatten, and saying so beats
/// quietly ignoring the flag.
#[test]
fn flatten_does_not_apply_to_a_bare_compressed_file() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let gz = dir.join("notes.txt.gz");
    let mut enc = flate2::write::GzEncoder::new(
        fs::File::create(&gz).unwrap(),
        flate2::Compression::default(),
    );
    enc.write_all(b"just text\n").unwrap();
    enc.finish().unwrap();

    let message = err(dir, &["extract", "notes.txt.gz", "out", "--flatten"]);
    assert!(message.contains("--flatten"), "{message}");
    assert!(message.contains("single file"), "{message}");
}

/// `--flatten` is not a way around the entry-name check: a name that climbs
/// out of `dest` is refused before anything is written, exactly as it is
/// without the flag.
#[test]
fn a_traversing_archive_is_still_refused_under_flatten() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let outside = dir.join("OWNED");

    let zip_path = dir.join("evil.zip");
    {
        let mut w = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        w.start_file::<_, ()>("../OWNED", Default::default())
            .unwrap();
        w.write_all(b"pwned").unwrap();
        w.finish().unwrap();
    }
    let message = err(dir, &["extract", "evil.zip", "unpacked", "--flatten"]);
    assert!(message.contains("OWNED"), "{message}");
    assert!(
        !outside.exists(),
        "zip escaped the destination under --flatten"
    );

    for entry in ["../OWNED", "/tmp/OWNED", "..\\OWNED", "pkg/../../OWNED"] {
        let tar_path = dir.join("evil.tar");
        fs::write(&tar_path, raw_tar(entry, b"pwned")).unwrap();
        let message = err(dir, &["extract", "evil.tar", "unpacked", "--flatten"]);
        assert!(message.contains("OWNED"), "{entry}: {message}");
        assert!(!outside.exists(), "{entry} escaped the destination");
    }
    assert!(!dir.join("unpacked").exists());
}

// ---------------------------------------------------------------------------
// path traversal
// ---------------------------------------------------------------------------

/// The pure rule, exercised directly: everything that could escape `dest` is
/// rejected, and everything ordinary survives.
#[test]
fn unsafe_entry_names_are_rejected() {
    for name in [
        "../evil",
        "a/../../evil",
        "/etc/passwd",
        "..\\evil",
        "a\\..\\..\\evil",
        "C:/Windows/evil",
        "C:evil",
        "\\\\server\\share\\evil",
    ] {
        assert!(
            pack::safe_entry(name, 0, false).is_err(),
            "{name} should have been rejected"
        );
    }

    assert_eq!(
        pack::safe_entry("a/b/c.txt", 0, false).unwrap(),
        Some(PathBuf::from("a").join("b").join("c.txt"))
    );
    // `.` and doubled separators are noise, not an escape.
    assert_eq!(
        pack::safe_entry("./a//b.txt", 0, false).unwrap(),
        Some(PathBuf::from("a").join("b.txt"))
    );
    // Stripped past its own depth: skipped, not an error.
    assert_eq!(pack::safe_entry("a/b.txt", 5, false).unwrap(), None);

    // Flattening happens after the same component check, so a name that would
    // escape is still refused, and what survives is a bare filename.
    for name in [
        "../evil",
        "a/../../evil",
        "/etc/passwd",
        "..\\evil",
        "C:evil",
    ] {
        assert!(
            pack::safe_entry(name, 0, true).is_err(),
            "{name} should have been rejected under --flatten"
        );
    }
    assert_eq!(
        pack::safe_entry("a/b/c.txt", 0, true).unwrap(),
        Some(PathBuf::from("c.txt"))
    );
    assert_eq!(
        pack::safe_entry("./a//b.txt", 0, true).unwrap(),
        Some(PathBuf::from("b.txt"))
    );
    assert_eq!(
        pack::safe_entry("lone.txt", 0, true).unwrap(),
        Some(PathBuf::from("lone.txt"))
    );
}

/// End to end, with archives built by hand the way an attacker would: the
/// extraction fails and nothing appears outside the destination.
#[test]
fn a_traversing_archive_writes_nothing_outside_dest() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let outside = dir.join("OWNED");

    // zip
    let zip_path = dir.join("evil.zip");
    {
        let mut w = zip::ZipWriter::new(fs::File::create(&zip_path).unwrap());
        w.start_file::<_, ()>("../OWNED", Default::default())
            .unwrap();
        w.write_all(b"pwned").unwrap();
        w.finish().unwrap();
    }
    let message = err(dir, &["extract", "evil.zip", "unpacked"]);
    assert!(message.contains("OWNED"), "{message}");
    assert!(!outside.exists(), "zip escaped the destination");

    // tar, including the absolute-path and Windows-separator variants. The
    // `tar` crate refuses to *write* these names, which is the whole reason
    // the header is assembled by hand here.
    for entry in ["../OWNED", "/tmp/OWNED", "..\\OWNED"] {
        let tar_path = dir.join("evil.tar");
        fs::write(&tar_path, raw_tar(entry, b"pwned")).unwrap();
        let message = err(dir, &["extract", "evil.tar", "unpacked"]);
        assert!(message.contains("OWNED"), "{entry}: {message}");
        assert!(!outside.exists(), "{entry} escaped the destination");
    }
}

/// A one-entry ustar archive with an arbitrary name, built byte by byte
/// because no well-behaved writer will produce one.
fn raw_tar(name: &str, body: &[u8]) -> Vec<u8> {
    let mut header = [b' '; 512];
    header[..100].fill(0);
    header[..name.len()].copy_from_slice(name.as_bytes());
    let put = |h: &mut [u8; 512], at: usize, text: &str| {
        h[at..at + text.len()].copy_from_slice(text.as_bytes());
        h[at + text.len()] = 0;
    };
    put(&mut header, 100, "0000644"); // mode
    put(&mut header, 108, "0000000"); // uid
    put(&mut header, 116, "0000000"); // gid
    put(&mut header, 124, &format!("{:011o}", body.len()));
    put(&mut header, 136, "00000000000"); // mtime
    header[156] = b'0'; // regular file
    header[157..257].fill(0); // linkname
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    header[265..500].fill(0);

    // The checksum is computed with its own field read as spaces.
    let sum: u32 = header.iter().map(|b| *b as u32).sum();
    put(&mut header, 148, &format!("{sum:06o}"));
    header[154] = b' ';

    let mut out = header.to_vec();
    out.extend_from_slice(body);
    out.resize(out.len().div_ceil(512) * 512, 0);
    out.extend_from_slice(&[0u8; 1024]); // two empty blocks end the archive
    out
}

/// A symlink pointing out of the tree is refused too: the OS, not the
/// extractor, would have followed it on the next write.
#[cfg(unix)]
#[test]
fn an_escaping_symlink_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let tar_path = dir.join("evil.tar");
    {
        let mut b = tar::Builder::new(fs::File::create(&tar_path).unwrap());
        let mut header = tar::Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_link_name("../../etc/passwd").unwrap();
        header.set_cksum();
        b.append_data(&mut header, "link", &[][..]).unwrap();
        b.finish().unwrap();
    }
    let message = err(dir, &["extract", "evil.tar", "unpacked"]);
    assert!(message.contains("outside"), "{message}");
    assert!(!dir.join("unpacked/link").exists());
}

// ---------------------------------------------------------------------------
// errors and --dry
// ---------------------------------------------------------------------------

#[test]
fn an_unknown_destination_extension_is_an_error() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    sample_tree(dir);
    let message = err(dir, &["archive", "src", "out.rar"]);
    assert!(message.contains("out.rar"), "{message}");
}

#[test]
fn an_unreadable_archive_is_an_error_naming_it() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    let message = err(dir, &["extract", "absent.zip", "unpacked"]);
    assert!(message.contains("absent.zip"), "{message}");
}

/// Under `--dry` both commands succeed, print nothing of their own, and leave
/// the filesystem exactly as it was.
#[test]
fn dry_does_nothing_and_says_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    sample_tree(dir);

    let (output, printed) = run(dir, true, &["archive", "src", "out.tar.gz"]).unwrap();
    assert!(output.success());
    assert_eq!(printed, "");
    assert!(!dir.join("out.tar.gz").exists());

    // Even against an archive that does not exist, so `--dry` never fails a
    // preview on a file an earlier step would have produced.
    let (output, printed) = run(dir, true, &["extract", "later.tar.gz", "unpacked"]).unwrap();
    assert!(output.success());
    assert_eq!(printed, "");
    assert!(!dir.join("unpacked").exists());
}

// ---------------------------------------------------------------------------
// format detection
// ---------------------------------------------------------------------------

#[test]
fn format_detection_matches_the_documented_extensions() {
    assert_eq!(pack::format_from_name("a.zip"), Some(Format::Zip));
    assert_eq!(pack::format_from_name("a.tar"), Some(Format::Tar));
    assert_eq!(
        pack::format_from_name("a.tar.zst"),
        Some(Format::Compressed(Codec::Zst))
    );
    assert_eq!(pack::format_from_name("a.out"), None);

    assert_eq!(
        pack::target_from_name("a.tar.xz"),
        Some(Target::Tar(Some(Codec::Xz)))
    );
    assert_eq!(pack::target_from_name("a.zip"), Some(Target::Zip));
    // `archive` writes containers only, so a bare codec has no meaning here.
    assert_eq!(pack::target_from_name("a.xz"), None);
}
