use std::error::Error;
use std::ffi::OsStr;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::Parser;
use globwalk::DirEntry;

#[derive(Parser)]
#[clap(author, version, about)]
struct Cli {
    /// The path to the Seatbelt compiler executable.
    #[clap(short, long, value_parser, default_value = "Seatbelt")]
    seatbelt_path: PathBuf,

    /// The path to the Upholsterer2k bssembler executable.
    #[clap(short, long, value_parser, default_value = "Upholsterer2k")]
    upholsterer_path: PathBuf,

    /// The path to the Backseater virtual machine executable.
    #[clap(short, long, value_parser, default_value = "backseat_safe_system_2k")]
    backseater_path: PathBuf,

    /// The path of the Backseat-scripts to test. The script files have to start with 'test_' and
    /// end with '.bs' to be tested.
    #[clap(short, long, value_parser, default_value = ".")]
    tests_path: PathBuf,
}

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    std::env::set_current_dir(&cli.tests_path)?;

    let _ = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "https://github.com/Backseating-Committee-2k/Seatbelt.git",
            "git_seatbelt_clone",
        ])
        .output()?;

    std::fs::rename("git_seatbelt_clone/std", "std")?;

    std::fs::remove_dir_all("git_seatbelt_clone")?;

    let mut tests_run: usize = 0;
    let mut tests_failed: usize = 0;
    for source_file in globwalk::glob("test*.bs")? {
        let source_file = source_file?;
        std::fs::rename(
            "std",
            source_file
                .path()
                .parent()
                .unwrap_or(&PathBuf::from("."))
                .join("std"),
        )?;
        let command_result = Command::new(cli.seatbelt_path.as_os_str())
            .arg(&source_file.path().as_os_str())
            .output()?;
        std::fs::rename(source_file.path().parent().unwrap().join("std"), "std")?;
        match command_result.status.success() {
            true => {
                let compiler_output = command_result.stdout;
                let upholsterer_result = child_with_pipe(&cli.upholsterer_path, compiler_output)?;
                match upholsterer_result.status.success() {
                    true => {
                        let backseater_result = child_with_pipe_args(
                            &cli.backseater_path,
                            upholsterer_result.stdout,
                            ["run"],
                        )?;
                        match backseater_result.status.success() {
                            true => eprintln!("TEST SUCCEEDED: {}", source_file.path().display()),
                            false => {
                                tests_failed += 1;
                                print_error(&source_file, backseater_result);
                            }
                        }
                    }
                    false => {
                        print_error(&source_file, upholsterer_result);
                        tests_failed += 1;
                    }
                }
            }
            false => {
                print_error(&source_file, command_result);
                tests_failed += 1;
            }
        }
        tests_run += 1;
    }
    println!(
        "Tests run: {}, Tests successful: {}, Tests failed: {}",
        tests_run,
        tests_run - tests_failed,
        tests_failed
    );
    std::fs::remove_dir_all("std")?;
    if tests_failed == 0 {
        Ok(())
    } else {
        Err("not all tests succeeded".into())
    }
}

fn child_with_pipe(
    path_of_executable: &Path,
    compiler_output: Vec<u8>,
) -> Result<std::process::Output, Box<dyn Error>> {
    let child = Command::new(path_of_executable.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    spawn_child(child, compiler_output)
}

fn child_with_pipe_args<S, I>(
    path_of_executable: &Path,
    compiler_output: Vec<u8>,
    args: I,
) -> Result<std::process::Output, Box<dyn Error>>
where
    S: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
{
    let child = Command::new(path_of_executable.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .args(args)
        .spawn()?;
    spawn_child(child, compiler_output)
}

fn spawn_child(
    mut child: std::process::Child,
    compiler_output: Vec<u8>,
) -> Result<std::process::Output, Box<dyn Error>> {
    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    std::thread::spawn(move || {
        stdin
            .write_all(&compiler_output)
            .expect("Failed to write to stdin");
    });
    Ok(child.wait_with_output()?)
}

fn print_error(source_file: &DirEntry, command_result: std::process::Output) {
    eprintln!("TEST FAILED: {}", source_file.path().display());
    let error_message = String::from_utf8_lossy(&command_result.stderr);
    let error_message = error_message.replace('\n', "\n\t");
    eprintln!("\t{error_message}");
}
