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

#[derive(Debug, PartialEq)]
enum TestOutcome {
    Finished,
    Aborted { error_message: String },
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
        let expected_outcome = determine_expected_outcome(source_file.path());
        if let Err(error) = expected_outcome {
            std::fs::remove_dir_all("std").ok();
            return Err(error);
        }
        let expected_outcome = expected_outcome.unwrap();

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
                            ["run", "--exit-on-halt"],
                        )?;
                        match backseater_result.status.success() {
                            true => {
                                if let TestOutcome::Aborted { error_message } = expected_outcome {
                                    eprintln!("TEST FAILED: {}", source_file.path().display());
                                    eprintln!("\ttest execution finished, but error message \"{}\" was expected",
                                error_message);
                                    tests_failed += 1;
                                } else {
                                    eprintln!("TEST SUCCEEDED: {}", source_file.path().display());
                                }
                            }
                            false => {
                                if let TestOutcome::Aborted { error_message } = expected_outcome {
                                    validate_error_message(
                                        &backseater_result,
                                        error_message,
                                        source_file.path(),
                                        &mut tests_failed,
                                    );
                                } else {
                                    tests_failed += 1;
                                    print_error(&source_file, backseater_result);
                                }
                            }
                        }
                    }
                    false => {
                        if let TestOutcome::Aborted { error_message } = expected_outcome {
                            validate_error_message(
                                &upholsterer_result,
                                error_message,
                                source_file.path(),
                                &mut tests_failed,
                            );
                        } else {
                            print_error(&source_file, upholsterer_result);
                            tests_failed += 1;
                        }
                    }
                }
            }
            false => {
                if let TestOutcome::Aborted { error_message } = expected_outcome {
                    validate_error_message(
                        &command_result,
                        error_message,
                        source_file.path(),
                        &mut tests_failed,
                    );
                } else {
                    print_error(&source_file, command_result);
                    tests_failed += 1;
                }
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

fn validate_error_message(
    command_result: &std::process::Output,
    error_message: String,
    source_file: &Path,
    tests_failed: &mut usize,
) {
    let stderr_string = String::from_utf8_lossy(&command_result.stderr);
    if stderr_string.contains(&error_message) {
        eprintln!("TEST SUCCEEDED: {}", source_file.display());
    } else {
        eprintln!("TEST FAILED: {}", source_file.display());
        eprintln!("\ttest aborted as expected, but with wrong error message:");
        eprintln!("\texpected: \"{}\"", error_message);
        eprintln!("\t     got: \"{}\"", stderr_string.trim());
        *tests_failed += 1;
    }
}

fn determine_expected_outcome(source_file: &Path) -> Result<TestOutcome, Box<dyn Error>> {
    let input_file = std::fs::read_to_string(source_file.as_os_str())?;
    let first_line = input_file.split('\n').next().unwrap().trim();
    if first_line.starts_with("//") {
        let test_runner_command = first_line.strip_prefix("//").unwrap().trim();
        let mut parts = test_runner_command.split('=');
        if let Some(lhs) = parts.next() {
            if let Some(rhs) = parts.next() {
                if lhs.trim() == "fails_with" {
                    let rhs = rhs.trim();
                    let rhs = rhs
                        .strip_prefix('"')
                        .ok_or_else(|| format!("\" prefix not found in {}", source_file.display()))?
                        .strip_suffix('"')
                        .ok_or_else(|| {
                            format!("\" suffix not found in {}", source_file.display())
                        })?;
                    return Ok(TestOutcome::Aborted {
                        error_message: String::from(rhs),
                    });
                }
            }
        }
    }
    Ok(TestOutcome::Finished)
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
