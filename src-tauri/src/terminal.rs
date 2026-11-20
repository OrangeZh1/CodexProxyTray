//! Only the dedicated directory below is added to the current user's PATH.
use std::{fs, io, path::Path};
use crate::{installation::{plain,sha256}, platform, settings};

const RECEIPT: &str = "terminal-shim-v1.json";

fn io_error(error: io::Error) -> String { format!("终端入口操作失败：{error}") }

fn prepare(root: &Path, source: &Path) -> Result<(), String> {
    let directory = root.join("terminal");
    let target = directory.join("codex.exe");
    let receipt = root.join(RECEIPT);
    let digest = sha256(source)?;
    plain(root)?;
    match fs::create_dir(&directory) {
        Err(error) if error.kind() != io::ErrorKind::AlreadyExists => return Err(io_error(error)),
        _ => {}
    }
    plain(&directory)?;
    for entry in fs::read_dir(&directory).map_err(io_error)? {
        if entry.map_err(io_error)?.file_name() != "codex.exe" {
            return Err("终端入口目录含其他文件，无法将它加入 PATH。".into());
        }
    }
    if target.try_exists().map_err(io_error)? {
        let current = sha256(&target)?;
        if current != digest {
            let owned: String = crate::installation::read_json(&receipt, 256)?;
            if owned != current { return Err("终端入口文件已改变，请检查后重试。".into()); }
        } else {
            return settings::write_json(&receipt, &digest);
        }
    }
    // Stage outside PATH. An interrupted replacement can be recovered from the
    // installed binary's hash even if the owned-file receipt was not yet written.
    let mut pending = tempfile::NamedTempFile::new_in(root).map_err(io_error)?;
    io::copy(&mut fs::File::open(source).map_err(io_error)?, pending.as_file_mut()).map_err(io_error)?;
    pending.as_file().sync_all().map_err(io_error)?;
    pending.persist(&target).map_err(|error| io_error(error.error))?;
    settings::write_json(&receipt, &digest)
}

pub fn enable(root: &Path, source: &Path) -> Result<(), String> {
    if settings::detect_cli(root).is_empty() { return Err("请先安装 Windows Codex CLI。".into()); }
    platform::check_machine_cli_path()?;
    prepare(root, source)?;
    platform::set_terminal_path(&root.join("terminal"), true)
}

pub fn disable(root: &Path) -> Result<(), String> {
    platform::set_terminal_path(&root.join("terminal"), false)
}

pub fn cleanup(root: &Path) -> Result<(), String> {
    disable(root)?;
    let directory = root.join("terminal");
    let target = directory.join("codex.exe");
    let receipt = root.join(RECEIPT);
    if !receipt.try_exists().map_err(io_error)? { return Ok(()); }
    let owned: String = crate::installation::read_json(&receipt, 256)?;
    if target.try_exists().map_err(io_error)? {
        if sha256(&target)? != owned { return Err("终端入口已改变，已保留该文件。".into()); }
        fs::remove_file(target).map_err(io_error)?;
    }
    if directory.try_exists().map_err(io_error)? { plain(&directory)?; fs::remove_dir(directory).map_err(io_error)?; }
    fs::remove_file(receipt).map_err(io_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{env, path::PathBuf, process::Command};

    #[test]
    fn alias_updates_owned_versions_and_preserves_other_files() {
        let temp=tempfile::tempdir().unwrap();
        let root=temp.path();
        let source=root.join("fixture.exe");
        fs::write(&source,b"v1").unwrap();
        prepare(root,&source).unwrap();
        fs::write(&source,b"v2").unwrap();
        prepare(root,&source).unwrap();
        let target=root.join("terminal/codex.exe");
        assert_eq!(fs::read(&target).unwrap(),b"v2");
        fs::write(&target,b"foreign").unwrap();
        assert!(prepare(root,&source).is_err());
        assert_eq!(fs::read(&target).unwrap(),b"foreign");
        fs::write(root.join("terminal/other.cmd"),b"keep").unwrap();
        assert!(prepare(root,&source).is_err());
    }

    #[test]
    #[ignore="Changes only a temporary path entry on an isolated GitHub-hosted Windows runner"]
    fn cmd_uses_opted_in_entry_and_restores_official_command() {
        assert_eq!(env::var("GITHUB_ACTIONS").as_deref(),Ok("true"));
        assert_eq!(env::var("RUNNER_ENVIRONMENT").as_deref(),Ok("github-hosted"));
        let temp=tempfile::tempdir_in(env::var_os("RUNNER_TEMP").unwrap()).unwrap();
        let node=Command::new("node").args(["-p","process.execPath"]).output().unwrap();
        assert!(node.status.success());
        let node=PathBuf::from(String::from_utf8(node.stdout).unwrap().trim());
        let source=PathBuf::from(env::var_os("CODEX_PROXY_TEST_SHIM").unwrap());
        let system=PathBuf::from(env::var_os("SystemRoot").unwrap()).join("System32");
        let cmd=system.join("cmd.exe");
        let roaming=temp.path().join("roaming");
        let npm=roaming.join("npm");
        let package=npm.join("node_modules/@openai/codex");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(package.join("package.json"),br#"{"name":"@openai/codex","bin":{"codex":"bin/codex.js"}}"#).unwrap();
        let script=package.join("bin/codex.js");
        let legacy_cli=include_str!("../../tests/mock-codex.mjs");
        fs::write(&script,legacy_cli).unwrap();
        fs::write(npm.join("codex.cmd"),b"@echo official\r\n").unwrap();
        env::set_var("LOCALAPPDATA",temp.path());env::set_var("APPDATA",&roaming);
        let root=settings::root();fs::create_dir(&root).unwrap();
        let dir=root.join("terminal");
        let base=env::join_paths([npm.as_path(),node.parent().unwrap(),system.as_path()]).unwrap();
        env::set_var("PATH",&base);
        let run=|path:&std::ffi::OsStr, args:&str| {
            let child=Command::new(&cmd).args(["/d","/c",args]).current_dir(temp.path())
                .env("PATH",path).env_remove("HTTP_PROXY").env_remove("HTTPS_PROXY")
                .env_remove("ALL_PROXY").env_remove("CODEX_PROXY_TRAY").output().unwrap();
            assert!(child.status.success(),"{}",String::from_utf8_lossy(&child.stderr));
            String::from_utf8(child.stdout).unwrap().trim().to_owned()
        };
        let original=platform::user_path_for_test();
        assert!(!platform::terminal_registered(&dir).unwrap());
        struct Restore(PathBuf);
        impl Drop for Restore {fn drop(&mut self){let _=disable(&self.0);}}
        let _restore=Restore(root.clone());
        assert_eq!(run(&base,"codex"),"official");
        enable(&root,&source).unwrap(); enable(&root,&source).unwrap();
        let (registered,_)=platform::user_path_for_test().unwrap();
        assert_eq!(registered.split(';').next(),dir.to_str());
        // Keep all other effective search locations inside the fixture.
        let effective=env::join_paths(std::iter::once(dir.clone()).chain(env::split_paths(&base))).unwrap();
        let event=platform::GuardEvent::new(&settings::event_name(&root)).unwrap();
        event.set(true);
        let read=|args:&str|serde_json::from_str::<serde_json::Value>(&run(&effective,args)).unwrap();
        assert_eq!(read("codex")["argv"],serde_json::json!([]));
        let help="if(process.argv[2]==='--help'){console.log('  --no-daemon  Run without the shared server');process.exit(0);}";
        fs::write(&script,format!("{help}\n{legacy_cli}")).unwrap();
        let guarded=read("codex");
        assert_eq!(guarded["argv"],serde_json::json!(["--no-daemon"]));
        assert_eq!(guarded["proxy"]["HTTPS_PROXY"],crate::core::DEFAULT_PROXY);
        assert_eq!(read("codex resume --last")["argv"],serde_json::json!(["--no-daemon","resume","--last"]));
        assert_eq!(read("codex -m demo hello")["argv"],serde_json::json!(["--no-daemon","-m","demo","hello"]));
        assert_eq!(read("codex --no-daemon")["argv"],serde_json::json!(["--no-daemon"]));
        event.set(false);
        assert!(read("codex")["proxy"]["HTTPS_PROXY"].is_null());
        assert_eq!(read("codex")["argv"],serde_json::json!([]));
        event.set(true);disable(&root).unwrap();
        assert!(!platform::terminal_registered(&dir).unwrap());
        assert!(read("codex")["proxy"]["HTTPS_PROXY"].is_null()); // stale terminal
        assert_eq!(run(&base,"codex"),"official");
        assert_eq!(platform::user_path_for_test().unwrap_or_default().0,original.unwrap_or_default().0);
        cleanup(&root).unwrap();assert!(!dir.exists());
    }
}
