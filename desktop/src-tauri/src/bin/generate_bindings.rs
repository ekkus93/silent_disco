use silent_disco_desktop_lib::bindings::render_typescript_bindings;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const GENERATED_RELATIVE_PATH: &str = "src/core/generated/desktop-bindings.ts";
const MAX_BINDING_BYTES: u64 = 1024 * 1024;

fn main() -> Result<(), Box<dyn Error>> {
    let check_only = parse_arguments()?;
    let output_path = output_path()?;
    let expected = render_typescript_bindings()?;
    if u64::try_from(expected.len())? > MAX_BINDING_BYTES {
        return Err("generated TypeScript bindings exceed the 1 MiB limit".into());
    }

    if check_only {
        check_existing(&output_path, expected.as_bytes())?;
    } else {
        write_generated(&output_path, expected.as_bytes())?;
        println!("generated {}", output_path.display());
    }
    Ok(())
}

fn parse_arguments() -> Result<bool, Box<dyn Error>> {
    let mut arguments = std::env::args_os();
    let _program = arguments.next();
    match (arguments.next(), arguments.next()) {
        (None, None) => Ok(false),
        (Some(argument), None) if argument == "--check" => Ok(true),
        _ => Err("usage: generate_bindings [--check]".into()),
    }
}

fn output_path() -> Result<PathBuf, Box<dyn Error>> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let desktop_root = manifest
        .parent()
        .ok_or("desktop Tauri manifest has no parent directory")?;
    Ok(desktop_root.join(GENERATED_RELATIVE_PATH))
}

fn check_existing(path: &Path, expected: &[u8]) -> Result<(), Box<dyn Error>> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("generated bindings are missing or unreadable: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("generated bindings path must be a regular file".into());
    }
    if metadata.len() > MAX_BINDING_BYTES {
        return Err("existing generated bindings exceed the 1 MiB limit".into());
    }

    let mut actual = Vec::new();
    File::open(path)?.take(MAX_BINDING_BYTES + 1).read_to_end(&mut actual)?;
    if actual != expected {
        return Err(format!(
            "generated TypeScript bindings are stale; run `npm run bindings:generate` and commit {}",
            path.display()
        )
        .into());
    }
    println!("verified {}", path.display());
    Ok(())
}

fn write_generated(path: &Path, content: &[u8]) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .ok_or("generated bindings path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let temporary = parent.join(format!(".desktop-bindings.ts.tmp-{}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;

    let write_result = file.write_all(content).and_then(|()| file.sync_all());
    drop(file);
    if let Err(primary) = write_result {
        let cleanup = fs::remove_file(&temporary);
        return match cleanup {
            Ok(()) => Err(primary.into()),
            Err(cleanup_error) => Err(format!(
                "binding write failed: {primary}; temporary cleanup also failed: {cleanup_error}"
            )
            .into()),
        };
    }

    match fs::rename(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            fs::remove_file(path)?;
            fs::rename(&temporary, path)?;
            Ok(())
        }
        Err(primary) => {
            let cleanup = fs::remove_file(&temporary);
            match cleanup {
                Ok(()) => Err(primary.into()),
                Err(cleanup_error) => Err(format!(
                    "binding publication failed: {primary}; temporary cleanup also failed: {cleanup_error}"
                )
                .into()),
            }
        }
    }
}
