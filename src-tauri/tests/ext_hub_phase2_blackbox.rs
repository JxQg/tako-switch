use base64::Engine;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use tako_switch_lib::config::{read_switch_state, save_switch_state, write_atomic, AppPaths};
use tako_switch_lib::extensions::skills::{
    align_skill_platforms_for_paths, discard_skill_zip_upload_for_paths,
    discover_repo_skills_for_paths, get_skills_hub_snapshot_for_paths,
    import_local_skill_for_paths, install_discovered_skill_for_paths, master_skills_dir,
    remove_skill_repo_for_paths, scan_adopt_local_skills_for_paths,
    set_skill_default_targets_for_paths, set_skill_sync_method_for_paths,
    stage_skill_zip_chunk_for_paths, toggle_skill_platform_for_paths, uninstall_skill_for_paths,
    update_installed_skill_for_paths, upsert_skill_repo_for_paths, ImportLocalSkillRequest,
    ManagedSkill, SkillLinkMode, SkillRepo, SkillSource, SkillSyncMethod, SkillsHubSnapshot,
};
use tako_switch_lib::extensions::{get_extensions_summary_for_paths, platform_skills_dirs, ExtPlatform};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use zip::write::SimpleFileOptions;

fn sandbox() -> (TempDir, AppPaths) {
    let directory = TempDir::new().unwrap();
    let paths = AppPaths::for_test(directory.path());
    assert!(paths.is_test);
    (directory, paths)
}

fn seed(path: &Path, relative: &str, content: &[u8]) {
    write_atomic(&path.join(relative), content, false).unwrap();
}

fn skill(path: &Path, name: &str, description: &str) {
    seed(
        path,
        "SKILL.md",
        format!("---\nname: {name}\ndescription: {description}\n---\n# Skill\n").as_bytes(),
    );
    seed(path, "scripts/run.txt", b"version-one");
}

fn managed<'a>(snapshot: &'a SkillsHubSnapshot, id: &str) -> &'a ManagedSkill {
    snapshot
        .skills
        .iter()
        .find(|item| item.skill.id == id)
        .unwrap()
}

fn master(paths: &AppPaths, id: &str) -> PathBuf {
    master_skills_dir(paths).join(id)
}

fn target(paths: &AppPaths, platform: ExtPlatform, id: &str) -> PathBuf {
    platform_skills_dirs(paths, platform)[0].join(id)
}

fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (path, bytes) in entries {
        writer
            .start_file(
                *path,
                SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn import_zip(
    paths: &AppPaths,
    bytes: &[u8],
    targets: Vec<ExtPlatform>,
) -> Result<SkillsHubSnapshot, String> {
    import_local_skill_for_paths(
        paths,
        ImportLocalSkillRequest {
            zip_base64: Some(base64::engine::general_purpose::STANDARD.encode(bytes)),
            targets: Some(targets),
            ..Default::default()
        },
    )
}

fn import_directory(paths: &AppPaths, path: &Path, targets: Vec<ExtPlatform>) -> SkillsHubSnapshot {
    import_local_skill_for_paths(
        paths,
        ImportLocalSkillRequest {
            path: Some(path.to_string_lossy().into_owned()),
            targets: Some(targets),
            ..Default::default()
        },
    )
    .unwrap()
}

fn assert_no_staging(root: &Path) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries {
            let entry = entry.unwrap();
            assert!(
                !entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".tako-skill-"),
                "stale transaction: {}",
                entry.path().display()
            );
            let metadata = fs::symlink_metadata(entry.path()).unwrap();
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                assert_no_staging(&entry.path());
            }
        }
    }
}

#[test]
fn tc_p2_01_adopts_all_platforms_and_uses_auto_or_filtered_atomic_copy() {
    let (_directory, paths) = sandbox();
    let extra = paths.home.join(".gemini/antigravity/skills");
    let cli = paths.home.join(".gemini/antigravity-cli/skills");
    fs::create_dir_all(&extra).unwrap();
    fs::create_dir_all(&cli).unwrap();
    skill(
        &target(&paths, ExtPlatform::Claude, "review"),
        "\"Review: Code\"",
        "'Review user''s code'",
    );
    skill(
        &target(&paths, ExtPlatform::Codex, "plan"),
        "'Build Plan'",
        "Plain text description",
    );
    skill(
        &target(&paths, ExtPlatform::Antigravity, "deploy"),
        "Deploy",
        "|\n  Deploy the application.\n  Keep its settings.",
    );
    skill(&extra.join("extra"), "Extra", "Extra CLI skill");
    skill(&cli.join("cli"), "CLI", "CLI only skill");
    let review = target(&paths, ExtPlatform::Claude, "review");
    for ignored in [".git", "node_modules", "__pycache__"] {
        seed(&review, &format!("{ignored}/ignored.txt"), b"ignored");
    }
    seed(&review, ".DS_Store", b"ignored");
    seed(&review, "assets/reference.txt", b"keep");
    seed(
        &paths.dsh_tako_dir,
        "skills/untouched/SKILL.md",
        b"do not touch",
    );

    let snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(snapshot.skills.len(), 5);
    assert_eq!(snapshot.sync_method, SkillSyncMethod::Auto);
    assert_eq!(snapshot.default_targets, ExtPlatform::all());
    assert_eq!(snapshot.repos[0].owner, "anthropics");
    assert_eq!(snapshot.repos[0].name, "skills");
    assert_eq!(snapshot.repos[0].branch, "main");
    assert!(snapshot.repos[0].enabled);
    assert_eq!(managed(&snapshot, "review").skill.name, "Review: Code");
    assert_eq!(
        managed(&snapshot, "review").skill.description,
        "Review user's code"
    );
    assert!(managed(&snapshot, "deploy")
        .skill
        .description
        .contains("Keep its settings."));
    for item in &snapshot.skills {
        assert_eq!(item.skill.source, SkillSource::Adopted);
        assert!(Path::new(&item.master_path).join("SKILL.md").is_file());
    }
    assert_eq!(
        managed(&snapshot, "cli").skill.enabled_platforms,
        [ExtPlatform::Antigravity]
    );
    assert!(!master(&paths, "review").join(".git").exists());
    assert!(!master(&paths, "review").join("node_modules").exists());
    assert!(!master(&paths, "review").join("__pycache__").exists());
    assert!(!master(&paths, "review").join(".DS_Store").exists());
    assert_eq!(
        fs::read(master(&paths, "review").join("assets/reference.txt")).unwrap(),
        b"keep"
    );

    let snapshot = align_skill_platforms_for_paths(&paths, "review", ExtPlatform::all()).unwrap();
    for platform in ExtPlatform::all() {
        let status = &managed(&snapshot, "review").platforms[platform];
        assert!(status.enabled);
        assert!(!status.has_diff);
        assert!(matches!(
            status.actual_method,
            Some(SkillLinkMode::Symlink | SkillLinkMode::Copy)
        ));
        #[cfg(unix)]
        assert_eq!(status.actual_method, Some(SkillLinkMode::Symlink));
    }
    assert!(extra.join("review/SKILL.md").is_file());
    assert!(cli.join("review/SKILL.md").is_file());
    let copied = set_skill_sync_method_for_paths(&paths, SkillSyncMethod::Copy).unwrap();
    for &platform in ExtPlatform::all() {
        assert_eq!(
            managed(&copied, "review").platforms[&platform].actual_method,
            Some(SkillLinkMode::Copy)
        );
        assert!(!fs::symlink_metadata(target(&paths, platform, "review"))
            .unwrap()
            .file_type()
            .is_symlink());
    }
    assert!(paths
        .backups_dir("extensions-skills")
        .join("claude/review/0/SKILL.md")
        .is_file());
    assert_eq!(
        fs::read(paths.dsh_tako_dir.join("skills/untouched/SKILL.md")).unwrap(),
        b"do not touch"
    );
    assert_no_staging(&paths.home);
}

#[test]
fn tc_p2_01_conflicting_history_is_adopted_without_overwrite_and_single_toggle_is_isolated() {
    let (_directory, paths) = sandbox();
    let claude = target(&paths, ExtPlatform::Claude, "shared");
    let codex = target(&paths, ExtPlatform::Codex, "shared");
    skill(&claude, "Shared", "Claude original");
    skill(&codex, "Shared", "Codex original");
    let codex_original = fs::read(codex.join("SKILL.md")).unwrap();
    let snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert!(managed(&snapshot, "shared").platforms[&ExtPlatform::Codex].has_diff);
    assert_eq!(fs::read(codex.join("SKILL.md")).unwrap(), codex_original);
    toggle_skill_platform_for_paths(&paths, "shared", ExtPlatform::Claude, false).unwrap();
    assert_eq!(fs::read(codex.join("SKILL.md")).unwrap(), codex_original);
    toggle_skill_platform_for_paths(&paths, "shared", ExtPlatform::Claude, true).unwrap();
    assert_eq!(fs::read(codex.join("SKILL.md")).unwrap(), codex_original);
    let rescanned = scan_adopt_local_skills_for_paths(&paths).unwrap();
    assert_eq!(rescanned.skills.len(), 1);
    assert!(managed(&rescanned, "shared").platforms[&ExtPlatform::Codex].has_diff);
}

#[test]
fn tc_p2_02_all_off_keeps_master_reenable_restores_and_uninstall_removes_every_target() {
    let (_directory, paths) = sandbox();
    fs::create_dir_all(paths.home.join(".gemini/antigravity/skills")).unwrap();
    fs::create_dir_all(paths.home.join(".gemini/antigravity-cli/skills")).unwrap();
    let source = paths.home.join("imports/retained");
    skill(&source, "retained", "Keep the master copy");
    let original = fs::read(source.join("SKILL.md")).unwrap();
    import_directory(&paths, &source, ExtPlatform::all().to_vec());
    for &platform in ExtPlatform::all() {
        let snapshot =
            toggle_skill_platform_for_paths(&paths, "retained", platform, false).unwrap();
        assert!(!managed(&snapshot, "retained").platforms[&platform].enabled);
        assert!(fs::symlink_metadata(target(&paths, platform, "retained")).is_err());
        assert_eq!(
            fs::read(master(&paths, "retained").join("SKILL.md")).unwrap(),
            original
        );
    }
    let all_off = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(all_off.skills.len(), 1);
    assert!(managed(&all_off, "retained")
        .skill
        .enabled_platforms
        .is_empty());
    let reopened =
        toggle_skill_platform_for_paths(&paths, "retained", ExtPlatform::Antigravity, true)
            .unwrap();
    assert!(managed(&reopened, "retained").platforms[&ExtPlatform::Antigravity].enabled);
    for root in platform_skills_dirs(&paths, ExtPlatform::Antigravity) {
        assert_eq!(fs::read(root.join("retained/SKILL.md")).unwrap(), original);
    }
    let uninstalled = uninstall_skill_for_paths(&paths, "retained").unwrap();
    assert!(uninstalled.skills.is_empty());
    assert!(!master(&paths, "retained").exists());
    for &platform in ExtPlatform::all() {
        for root in platform_skills_dirs(&paths, platform) {
            assert!(fs::symlink_metadata(root.join("retained")).is_err());
        }
    }
    assert!(source.join("SKILL.md").is_file());
    assert!(get_skills_hub_snapshot_for_paths(&paths)
        .unwrap()
        .skills
        .is_empty());
    assert_no_staging(&paths.home);
}

#[cfg(unix)]
#[test]
fn tc_p2_02_unix_removes_directory_links_without_following_their_targets() {
    let (_directory, paths) = sandbox();
    let source = paths.home.join("imports/linked");
    skill(&source, "linked", "Unix link");
    set_skill_sync_method_for_paths(&paths, SkillSyncMethod::Symlink).unwrap();
    import_directory(&paths, &source, vec![ExtPlatform::Codex]);
    let destination = target(&paths, ExtPlatform::Codex, "linked");
    assert!(fs::symlink_metadata(&destination)
        .unwrap()
        .file_type()
        .is_symlink());
    toggle_skill_platform_for_paths(&paths, "linked", ExtPlatform::Codex, false).unwrap();
    assert!(master(&paths, "linked").join("scripts/run.txt").is_file());
    let outside = paths.home.join("outside");
    skill(&outside, "outside", "Must stay");
    std::os::unix::fs::symlink(&outside, &destination).unwrap();
    toggle_skill_platform_for_paths(&paths, "linked", ExtPlatform::Codex, false).unwrap();
    assert!(outside.join("scripts/run.txt").is_file());
}

#[cfg(windows)]
#[test]
fn tc_p2_02_windows_removes_junction_without_deleting_master_files() {
    use std::os::windows::fs::{FileTypeExt, MetadataExt};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    let (_directory, paths) = sandbox();
    let source = paths.home.join("imports/junction");
    skill(&source, "junction", "Windows junction");
    import_directory(&paths, &source, Vec::new());
    let destination = target(&paths, ExtPlatform::Codex, "junction");
    fs::create_dir_all(destination.parent().unwrap()).unwrap();
    let status = Command::new("cmd.exe")
        .args(["/d", "/c", "mklink", "/J"])
        .arg(&destination)
        .arg(master(&paths, "junction"))
        .creation_flags(0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
    assert_ne!(
        fs::symlink_metadata(&destination)
            .unwrap()
            .file_attributes()
            & 0x400,
        0
    );
    let snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(
        managed(&snapshot, "junction").platforms[&ExtPlatform::Codex].actual_method,
        Some(SkillLinkMode::Symlink)
    );
    toggle_skill_platform_for_paths(&paths, "junction", ExtPlatform::Codex, false).unwrap();
    assert!(fs::symlink_metadata(&destination).is_err());
    assert!(master(&paths, "junction").join("scripts/run.txt").is_file());
    if std::os::windows::fs::symlink_dir(master(&paths, "junction"), &destination).is_ok() {
        assert!(fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink_dir());
        toggle_skill_platform_for_paths(&paths, "junction", ExtPlatform::Codex, false).unwrap();
        assert!(master(&paths, "junction").join("SKILL.md").is_file());
    }
}

#[test]
fn tc_p2_03_imports_local_root_zip_nested_multi_zip_and_persists_default_targets() {
    let (_directory, paths) = sandbox();
    set_skill_sync_method_for_paths(&paths, SkillSyncMethod::Copy).unwrap();
    set_skill_default_targets_for_paths(&paths, &[ExtPlatform::Codex]).unwrap();
    let source = paths.home.join("imports/local-skill");
    skill(&source, "'Local Skill'", "\"A description: with colon\"");
    let local = import_local_skill_for_paths(
        &paths,
        ImportLocalSkillRequest {
            path: Some(source.to_string_lossy().into_owned()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(managed(&local, "local-skill").skill.name, "Local Skill");
    assert_eq!(
        managed(&local, "local-skill").skill.description,
        "A description: with colon"
    );
    assert_eq!(
        managed(&local, "local-skill").skill.enabled_platforms,
        [ExtPlatform::Codex]
    );
    let root_zip = zip_bytes(&[
        (
            "SKILL.md",
            b"---\nname: root-zip\ndescription: 'Root ZIP'\n---\n# Root",
        ),
        ("assets/example.txt", b"root-asset"),
    ]);
    let root = import_zip(&paths, &root_zip, vec![ExtPlatform::Claude]).unwrap();
    assert_eq!(managed(&root, "root-zip").skill.source, SkillSource::Zip);
    assert!(target(&paths, ExtPlatform::Claude, "root-zip")
        .join("assets/example.txt")
        .is_file());
    let multi = zip_bytes(&[
        ("bundle/first/SKILL.md", b"---\nname: first\ndescription: First skill\n---\n# First"),
        ("bundle/first/scripts/tool.txt", b"first-script"),
        ("bundle/second/SKILL.md", b"---\nname: second\ndescription: >-\n  Second skill\n  folded description\n---\n# Second"),
        ("bundle/second/node_modules/ignored.txt", b"ignored"),
        ("bundle/second/__pycache__/ignored.pyc", b"ignored"),
        ("README.md", b"not a skill"),
    ]);
    let archive = paths.home.join("imports/bundle.zip");
    write_atomic(&archive, &multi, false).unwrap();
    let result = import_local_skill_for_paths(
        &paths,
        ImportLocalSkillRequest {
            path: Some(archive.to_string_lossy().into_owned()),
            targets: Some(ExtPlatform::all().to_vec()),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(result.skills.len(), 4);
    assert_eq!(
        managed(&result, "second").skill.description,
        "Second skill folded description"
    );
    assert!(!master(&paths, "second").join("node_modules").exists());
    assert!(!master(&paths, "second").join("__pycache__").exists());
    assert_eq!(
        fs::read(master(&paths, "first").join("scripts/tool.txt")).unwrap(),
        b"first-script"
    );
    assert_eq!(
        get_skills_hub_snapshot_for_paths(&paths)
            .unwrap()
            .default_targets,
        [ExtPlatform::Codex]
    );
    assert_no_staging(&paths.home);
}

#[test]
fn tc_p2_03_rejects_zip_slip_absolute_drive_duplicate_and_symlink_entries_before_writes() {
    let (_directory, paths) = sandbox();
    for attack in [
        "../evil",
        "safe/../../evil",
        "/evil",
        "C:/evil",
        "C:evil",
        "..\\evil",
        "safe\\..\\evil",
        "safe/./evil",
        "safe//evil",
        "safe/CON.txt",
    ] {
        let archive = zip_bytes(&[("valid/SKILL.md", b"# Valid"), (attack, b"evil")]);
        let error = import_zip(&paths, &archive, ExtPlatform::all().to_vec()).unwrap_err();
        assert!(error.contains("Zip Slip"), "{attack}: {error}");
        assert!(!master(&paths, "valid").exists());
        assert!(!paths.home.join("evil").exists());
    }
    let duplicate = zip_bytes(&[
        ("safe/SKILL.md", b"# Safe"),
        ("safe/scripts/A.txt", b"a"),
        ("safe/scripts/a.txt", b"b"),
    ]);
    assert!(import_zip(&paths, &duplicate, Vec::new())
        .unwrap_err()
        .contains("重复"));
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .start_file("safe/SKILL.md", SimpleFileOptions::default())
        .unwrap();
    writer.write_all(b"# Safe").unwrap();
    writer
        .add_symlink("safe/escape", "../../outside", SimpleFileOptions::default())
        .unwrap();
    let archive = writer.finish().unwrap().into_inner();
    assert!(import_zip(&paths, &archive, Vec::new())
        .unwrap_err()
        .contains("链接"));
    assert!(!master(&paths, "safe").exists());
    assert_no_staging(&paths.home);
}

#[test]
fn tc_p2_03_size_depth_invalid_frontmatter_and_batch_collisions_do_not_partially_import() {
    let (_directory, paths) = sandbox();
    let chunk_50m = vec![b'x'; 50 * 1024 * 1024];
    let archive = zip_bytes(&[
        ("huge/SKILL.md", b"# Huge"),
        ("huge/a.bin", &chunk_50m),
        ("huge/b.bin", &chunk_50m),
        ("huge/c.bin", &chunk_50m),
        ("huge/d.bin", &chunk_50m),
    ]);
    assert!(import_zip(&paths, &archive, Vec::new())
        .unwrap_err()
        .contains("200 MiB"));
    let deep = format!("{}SKILL.md", "nested/".repeat(18));
    assert!(import_zip(&paths, &zip_bytes(&[(&deep, b"# Deep")]), Vec::new()).is_err());
    let invalid = zip_bytes(&[
        ("valid/SKILL.md", b"# Valid"),
        ("bad/SKILL.md", b"---\nname: [invalid\n---\n# Bad"),
    ]);
    assert!(import_zip(&paths, &invalid, Vec::new())
        .unwrap_err()
        .contains("frontmatter"));
    assert!(!master(&paths, "valid").exists());
    let source = paths.home.join("imports/existing");
    skill(&source, "existing", "Original");
    import_directory(&paths, &source, Vec::new());
    let original = fs::read(master(&paths, "existing").join("SKILL.md")).unwrap();
    let collision = zip_bytes(&[
        ("fresh/SKILL.md", b"# Fresh"),
        ("existing/SKILL.md", b"# Overwrite"),
    ]);
    assert!(import_zip(&paths, &collision, ExtPlatform::all().to_vec())
        .unwrap_err()
        .contains("已存在"));
    assert!(!master(&paths, "fresh").exists());
    assert_eq!(
        fs::read(master(&paths, "existing").join("SKILL.md")).unwrap(),
        original
    );
    assert_no_staging(&paths.home);
}

#[test]
fn partial_metadata_and_missing_stores_default_without_losing_disabled_skills() {
    let (_directory, paths) = sandbox();
    seed(&paths.switch_dir(), "extensions/skills.json", br#"{}"#);
    seed(&paths.switch_dir(), "extensions/skill_repos.json", br#"{}"#);
    let source = paths.home.join("imports/disabled");
    skill(&source, "disabled", "Initially disabled");
    let snapshot = import_directory(&paths, &source, Vec::new());
    assert_eq!(snapshot.sync_method, SkillSyncMethod::Auto);
    assert_eq!(snapshot.repos[0].owner, "anthropics");
    assert!(managed(&snapshot, "disabled")
        .skill
        .enabled_platforms
        .is_empty());
    assert!(get_skills_hub_snapshot_for_paths(&paths)
        .unwrap()
        .skills
        .iter()
        .any(|item| item.skill.id == "disabled"));
    seed(
        &paths.switch_dir(),
        "extensions/skills.json",
        br#"{"skills":[{"id":"disabled"}]}"#,
    );
    assert_eq!(
        get_skills_hub_snapshot_for_paths(&paths)
            .unwrap()
            .skills
            .len(),
        1
    );
    seed(&paths.switch_dir(), "extensions/skills.json", b"not-json");
    assert!(get_skills_hub_snapshot_for_paths(&paths).is_err());
    assert_eq!(
        fs::read(paths.switch_dir().join("extensions/skills.json")).unwrap(),
        b"not-json"
    );
}

#[test]
fn identifiers_and_failed_distribution_preserve_source_and_master() {
    let (_directory, paths) = sandbox();
    let source = paths.home.join("imports/blocked");
    skill(&source, "blocked", "Do not lose files");
    for invalid in ["../outside", "C:outside", "bad/path", "..", "CON", "-flag"] {
        assert!(uninstall_skill_for_paths(&paths, invalid).is_err());
        assert!(
            toggle_skill_platform_for_paths(&paths, invalid, ExtPlatform::Codex, true).is_err()
        );
    }
    seed(&paths.codex_dir, "skills", b"not a directory");
    let error = import_local_skill_for_paths(
        &paths,
        ImportLocalSkillRequest {
            path: Some(source.to_string_lossy().into_owned()),
            targets: Some(ExtPlatform::all().to_vec()),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("目录"));
    assert!(!master(&paths, "blocked").exists());
    assert!(!target(&paths, ExtPlatform::Claude, "blocked").exists());
    assert!(source.join("SKILL.md").is_file());
    assert_no_staging(&paths.home);
}

#[test]
fn importing_an_existing_client_skill_requires_adoption_without_overwriting_it() {
    let (_directory, paths) = sandbox();
    let source = paths.home.join("imports/existing-client");
    let existing = target(&paths, ExtPlatform::Claude, "existing-client");
    skill(&source, "existing-client", "Incoming content");
    skill(&existing, "existing-client", "Original client content");
    let original = fs::read(existing.join("SKILL.md")).unwrap();
    let error = import_local_skill_for_paths(
        &paths,
        ImportLocalSkillRequest {
            path: Some(source.to_string_lossy().into_owned()),
            targets: Some(vec![ExtPlatform::Codex]),
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(error.contains("扫描收编"));
    assert_eq!(fs::read(existing.join("SKILL.md")).unwrap(), original);
    assert!(!master(&paths, "existing-client").exists());
    let snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(
        managed(&snapshot, "existing-client").skill.source,
        SkillSource::Adopted
    );
}

struct MockArchive {
    base: String,
    body: Arc<RwLock<Vec<u8>>>,
    paths: Arc<RwLock<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl MockArchive {
    async fn start(body: Vec<u8>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let body = Arc::new(RwLock::new(body));
        let paths = Arc::new(RwLock::new(Vec::new()));
        let server_body = body.clone();
        let server_paths = paths.clone();
        let task = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let body = server_body.clone();
                let paths = server_paths.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0u8; 2048];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        let read = stream.read(&mut buffer).await.unwrap();
                        if read == 0 {
                            return;
                        }
                        request.extend_from_slice(&buffer[..read]);
                    }
                    let first = String::from_utf8_lossy(&request)
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string();
                    paths.write().unwrap().push(first.clone());
                    let missing = first.contains("/main.zip");
                    let bytes = if missing {
                        Vec::new()
                    } else {
                        body.read().unwrap().clone()
                    };
                    let status = if missing { "404 Not Found" } else { "200 OK" };
                    let headers = format!("HTTP/1.1 {status}\r\nContent-Type: application/zip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", bytes.len());
                    stream.write_all(headers.as_bytes()).await.unwrap();
                    stream.write_all(&bytes).await.unwrap();
                });
            }
        });
        Self {
            base,
            body,
            paths,
            task,
        }
    }

    fn repo(&self) -> SkillRepo {
        SkillRepo {
            owner: "test-owner".to_string(),
            name: "test-skills".to_string(),
            branch: "main".to_string(),
            enabled: true,
            archive_url_override: Some(format!("{}/{{branch}}.zip", self.base)),
        }
    }
}

impl Drop for MockArchive {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn repo_zip(version: &[u8], reverse_order: bool) -> Vec<u8> {
    let mut entries: Vec<(&str, &[u8])> = vec![
        (
            "test-skills-master/skills/repo-skill/SKILL.md",
            b"---\nname: 'Repo Skill'\ndescription: \"From archive\"\n---\n# Repository",
        ),
        (
            "test-skills-master/skills/repo-skill/scripts/run.txt",
            version,
        ),
        (
            "test-skills-master/skills/another/SKILL.md",
            b"---\nname: another\ndescription: Another skill\n---\n# Another",
        ),
        ("test-skills-master/README.md", b"repository root"),
    ];
    if reverse_order {
        entries.reverse();
    }
    zip_bytes(&entries)
}

#[tokio::test]
async fn tc_p2_04_mock_repo_discovery_main_fallback_install_hash_change_update_and_remove() {
    let (_directory, paths) = sandbox();
    set_skill_sync_method_for_paths(&paths, SkillSyncMethod::Copy).unwrap();
    let server = MockArchive::start(repo_zip(b"remote-v1", false)).await;
    upsert_skill_repo_for_paths(&paths, server.repo()).unwrap();
    let found = discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
        .await
        .unwrap();
    assert_eq!(found.len(), 2);
    let discovered = found.iter().find(|skill| skill.id == "repo-skill").unwrap();
    assert_eq!(discovered.repo_branch, "master");
    assert_eq!(discovered.repo_subdir, "skills/repo-skill");
    assert_eq!(discovered.name, "Repo Skill");
    assert!(discovered.installed_id.is_none());
    assert!(server
        .paths
        .read()
        .unwrap()
        .iter()
        .any(|path| path.contains("/main.zip")));
    assert!(server
        .paths
        .read()
        .unwrap()
        .iter()
        .any(|path| path.contains("/master.zip")));
    let installed = install_discovered_skill_for_paths(
        &paths,
        "test-owner",
        "test-skills",
        &discovered.repo_subdir,
        Some(vec![ExtPlatform::Codex, ExtPlatform::Antigravity]),
    )
    .await
    .unwrap();
    let record = &managed(&installed, "repo-skill").skill;
    assert_eq!(record.source, SkillSource::Github);
    assert_eq!(record.repo_owner.as_deref(), Some("test-owner"));
    assert_eq!(record.repo_branch.as_deref(), Some("master"));
    assert_eq!(record.installed_hash, discovered.remote_hash);
    assert!(!target(&paths, ExtPlatform::Claude, "repo-skill").exists());
    let original_hash = record.installed_hash.clone();
    *server.body.write().unwrap() = repo_zip(b"remote-v1", true);
    let same = discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
        .await
        .unwrap();
    let same = same.iter().find(|skill| skill.id == "repo-skill").unwrap();
    assert_eq!(
        same.remote_hash, original_hash,
        "ZIP entry order must not affect stable content hash"
    );
    assert!(!same.update_available);
    *server.body.write().unwrap() = repo_zip(b"remote-v2", false);
    let changed = discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
        .await
        .unwrap();
    assert!(
        changed
            .iter()
            .find(|skill| skill.id == "repo-skill")
            .unwrap()
            .update_available
    );
    let snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert!(managed(&snapshot, "repo-skill").skill.update_available);
    let summary_before_update = get_extensions_summary_for_paths(&paths).unwrap();
    assert_eq!(summary_before_update.skills_count, 1);
    assert_eq!(summary_before_update.skills_updatable_count, 1);
    assert_eq!(
        managed(&snapshot, "repo-skill").skill.installed_hash,
        original_hash
    );
    let updated = update_installed_skill_for_paths(&paths, "repo-skill")
        .await
        .unwrap();
    assert!(!managed(&updated, "repo-skill").skill.update_available);
    let summary_after_update = get_extensions_summary_for_paths(&paths).unwrap();
    assert_eq!(summary_after_update.skills_updatable_count, 0);
    assert_ne!(
        managed(&updated, "repo-skill").skill.installed_hash,
        original_hash
    );
    assert_eq!(
        fs::read(master(&paths, "repo-skill").join("scripts/run.txt")).unwrap(),
        b"remote-v2"
    );
    for platform in [ExtPlatform::Codex, ExtPlatform::Antigravity] {
        assert_eq!(
            fs::read(target(&paths, platform, "repo-skill").join("scripts/run.txt")).unwrap(),
            b"remote-v2"
        );
    }
    assert!(!target(&paths, ExtPlatform::Claude, "repo-skill").exists());
    let without_repo = remove_skill_repo_for_paths(&paths, "test-owner", "test-skills").unwrap();
    assert!(!without_repo
        .repos
        .iter()
        .any(|repo| repo.owner == "test-owner"));
    assert_eq!(without_repo.skills.len(), 1);
    assert!(
        discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
            .await
            .is_err()
    );
    assert_no_staging(&paths.home);
}

#[tokio::test]
async fn tc_p2_04_preserves_local_modifications_and_rejects_unsafe_remote_archive() {
    let (_directory, paths) = sandbox();
    set_skill_sync_method_for_paths(&paths, SkillSyncMethod::Copy).unwrap();
    let server = MockArchive::start(repo_zip(b"remote-v1", false)).await;
    upsert_skill_repo_for_paths(&paths, server.repo()).unwrap();
    install_discovered_skill_for_paths(
        &paths,
        "test-owner",
        "test-skills",
        "skills/repo-skill",
        Some(vec![ExtPlatform::Claude]),
    )
    .await
    .unwrap();
    *server.body.write().unwrap() = repo_zip(b"remote-v2", false);
    discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
        .await
        .unwrap();
    seed(
        &master(&paths, "repo-skill"),
        "scripts/run.txt",
        b"local-change",
    );
    assert!(update_installed_skill_for_paths(&paths, "repo-skill")
        .await
        .unwrap_err()
        .contains("本地修改"));
    assert_eq!(
        fs::read(master(&paths, "repo-skill").join("scripts/run.txt")).unwrap(),
        b"local-change"
    );
    seed(
        &master(&paths, "repo-skill"),
        "scripts/run.txt",
        b"remote-v1",
    );
    seed(
        &target(&paths, ExtPlatform::Claude, "repo-skill"),
        "scripts/run.txt",
        b"client-change",
    );
    assert!(update_installed_skill_for_paths(&paths, "repo-skill")
        .await
        .unwrap_err()
        .contains("本地修改"));
    *server.body.write().unwrap() = zip_bytes(&[
        ("test-skills-master/skills/unsafe/SKILL.md", b"# Unsafe"),
        ("../evil", b"evil"),
    ]);
    assert!(
        discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
            .await
            .unwrap_err()
            .contains("Zip Slip")
    );
    assert!(master(&paths, "repo-skill").join("SKILL.md").is_file());
    assert!(!paths.home.join("evil").exists());
    assert_no_staging(&paths.home);
}

#[tokio::test]
async fn tc_p2_04_repo_download_honors_live_proxy_policy_using_local_http_only() {
    struct ProxyEnvironment(Vec<(&'static str, Option<std::ffi::OsString>)>);
    impl Drop for ProxyEnvironment {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                if let Some(value) = value {
                    std::env::set_var(key, value);
                } else {
                    std::env::remove_var(key);
                }
            }
        }
    }
    let (_directory, paths) = sandbox();
    let server = MockArchive::start(repo_zip(b"proxy-version", false)).await;
    let keys = ["HTTP_PROXY", "http_proxy", "NO_PROXY", "no_proxy"];
    let _environment = ProxyEnvironment(
        keys.iter()
            .map(|&key| (key, std::env::var_os(key)))
            .collect(),
    );
    std::env::set_var("HTTP_PROXY", &server.base);
    std::env::set_var("http_proxy", &server.base);
    std::env::remove_var("NO_PROXY");
    std::env::remove_var("no_proxy");
    let direct = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let direct_address = direct.local_addr().unwrap();
    let direct_task = tokio::spawn(async move {
        while let Ok((stream, _)) = direct.accept().await {
            drop(stream);
        }
    });
    let mut repo = server.repo();
    repo.branch = "master".to_string();
    repo.archive_url_override = Some(format!("http://{direct_address}/archive.zip"));
    upsert_skill_repo_for_paths(&paths, repo).unwrap();
    let mut state = read_switch_state(&paths);
    assert!(state.bypass_system_proxy);
    assert!(
        discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
            .await
            .is_err()
    );
    assert!(server.paths.read().unwrap().is_empty());
    state.bypass_system_proxy = false;
    save_switch_state(&paths, &state).unwrap();
    let found = discover_repo_skills_for_paths(&paths, "test-owner", "test-skills")
        .await
        .unwrap();
    assert_eq!(found.len(), 2);
    assert!(server
        .paths
        .read()
        .unwrap()
        .iter()
        .any(|request| request.starts_with("GET http://")));
    direct_task.abort();
}

#[test]
fn tc_p2_05_adopts_over_50mb_skill_keeps_dist_skips_env_caches_and_decouples_home_summary() {
    let (_directory, paths) = sandbox();
    let external_agents = paths.home.join(".agents/skills/dashi-ppt");
    seed(
        &external_agents,
        "SKILL.md",
        b"---\nname: dashi-ppt\ndescription: Heavy offline template skill\n---\n# Dashi\n",
    );
    let dist_payload = vec![b'd'; 28 * 1024 * 1024];
    let src_payload = vec![b's'; 28 * 1024 * 1024];
    seed(&external_agents, "project/dist/bundle.js", &dist_payload);
    seed(&external_agents, "project/src/templates.bin", &src_payload);
    for ignored in [".venv", "venv", ".pytest_cache", ".mypy_cache", ".ruff_cache"] {
        seed(&external_agents, &format!("{ignored}/cache.bin"), b"skip-me");
    }
    seed(&external_agents, "Thumbs.db", b"skip-thumbs");

    let codex_skill = target(&paths, ExtPlatform::Codex, "dashi-ppt");
    fs::create_dir_all(codex_skill.parent().unwrap()).unwrap();
    #[cfg(unix)]
    let linked_external = std::os::unix::fs::symlink(&external_agents, &codex_skill).is_ok();
    #[cfg(windows)]
    let linked_external = std::os::windows::fs::symlink_dir(&external_agents, &codex_skill).is_ok();
    if !linked_external {
        seed(
            &codex_skill,
            "SKILL.md",
            b"---\nname: dashi-ppt\ndescription: Heavy offline template skill\n---\n# Dashi\n",
        );
        seed(&codex_skill, "project/dist/bundle.js", &dist_payload);
        seed(&codex_skill, "project/src/templates.bin", &src_payload);
        seed(&codex_skill, ".venv/cache.bin", b"skip-me");
        seed(&codex_skill, "Thumbs.db", b"skip-thumbs");
    }

    // 1. Home summary must be zero-copy lightweight and never fail on 56 MiB skill
    let summary = get_extensions_summary_for_paths(&paths).unwrap();
    assert_eq!(summary.skills_count, 1);
    assert!(
        !master(&paths, "dashi-ppt").exists(),
        "home summary should not perform heavy file copy adoption"
    );

    // 2. Entering Skills Hub streams 56 MiB skill into master, keeps dist/, filters env caches, preserves external symlink
    let snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert!(snapshot.warnings.is_empty(), "{:?}", snapshot.warnings);
    assert_eq!(snapshot.skills.len(), 1);
    let item = managed(&snapshot, "dashi-ppt");
    assert_eq!(item.skill.source, SkillSource::Adopted);
    assert!(!item.platforms[&ExtPlatform::Codex].has_diff);
    if linked_external {
        assert_eq!(
            item.platforms[&ExtPlatform::Codex].actual_method,
            Some(SkillLinkMode::Symlink)
        );
        assert_eq!(
            fs::canonicalize(&codex_skill).unwrap(),
            fs::canonicalize(&external_agents).unwrap(),
            "passive adoption must preserve existing external symlink"
        );
    }
    assert_eq!(
        fs::metadata(master(&paths, "dashi-ppt").join("project/dist/bundle.js"))
            .unwrap()
            .len(),
        dist_payload.len() as u64
    );
    assert!(!master(&paths, "dashi-ppt").join(".venv").exists());
    assert!(!master(&paths, "dashi-ppt").join("venv").exists());
    assert!(!master(&paths, "dashi-ppt").join(".pytest_cache").exists());
    assert!(!master(&paths, "dashi-ppt").join("Thumbs.db").exists());
    assert_no_staging(&paths.home);
}

#[test]
fn tc_p2_06_isolates_corrupted_skills_with_warnings_without_breaking_hub_or_summary() {
    let (_directory, paths) = sandbox();
    skill(
        &target(&paths, ExtPlatform::Claude, "healthy"),
        "Healthy Skill",
        "Works cleanly",
    );
    seed(
        &target(&paths, ExtPlatform::Codex, "broken-yaml"),
        "SKILL.md",
        b"---\nname: [unclosed\n---\n# Broken",
    );
    seed(
        &target(&paths, ExtPlatform::Antigravity, "invalid space"),
        "SKILL.md",
        b"# Invalid folder identifier",
    );

    let summary = get_extensions_summary_for_paths(&paths).unwrap();
    assert!(summary.skills_count >= 1);

    let snapshot = get_skills_hub_snapshot_for_paths(&paths).unwrap();
    assert_eq!(snapshot.skills.len(), 1);
    assert_eq!(snapshot.skills[0].skill.id, "healthy");
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|w| w.contains("broken-yaml") && w.contains("frontmatter")),
        "expected broken-yaml warning in {:?}",
        snapshot.warnings
    );
    assert!(
        snapshot
            .warnings
            .iter()
            .any(|w| w.contains("invalid space")),
        "expected invalid identifier warning in {:?}",
        snapshot.warnings
    );
    assert!(!master(&paths, "broken-yaml").exists());
    assert_no_staging(&paths.home);
}

#[test]
fn tc_p2_07_chunked_zip_upload_installs_and_cleans_up_temp_files() {
    let (_directory, paths) = sandbox();
    let archive = zip_bytes(&[
        (
            "chunked-skill/SKILL.md",
            b"---\nname: Chunked Skill\ndescription: Uploaded in slices\n---\n# Chunked",
        ),
        ("chunked-skill/assets/data.txt", b"slice-payload"),
    ]);
    let mid = archive.len() / 2;
    let c1 = base64::engine::general_purpose::STANDARD.encode(&archive[..mid]);
    let c2 = base64::engine::general_purpose::STANDARD.encode(&archive[mid..]);

    let upload_id = stage_skill_zip_chunk_for_paths(&paths, None, &c1).unwrap();
    let same_id = stage_skill_zip_chunk_for_paths(&paths, Some(&upload_id), &c2).unwrap();
    assert_eq!(upload_id, same_id);

    let snapshot = import_local_skill_for_paths(
        &paths,
        ImportLocalSkillRequest {
            upload_id: Some(upload_id),
            targets: Some(vec![ExtPlatform::Codex]),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(managed(&snapshot, "chunked-skill").skill.name, "Chunked Skill");
    assert_eq!(
        fs::read(target(&paths, ExtPlatform::Codex, "chunked-skill").join("assets/data.txt"))
            .unwrap(),
        b"slice-payload"
    );
    assert!(!paths
        .switch_dir()
        .join("extensions/.tmp-uploads")
        .exists());

    let discarded = stage_skill_zip_chunk_for_paths(&paths, None, &c1).unwrap();
    discard_skill_zip_upload_for_paths(&paths, &discarded).unwrap();
    assert!(!paths
        .switch_dir()
        .join("extensions/.tmp-uploads")
        .exists());
}
