use std::error::Error;
use std::ffi::OsStr;
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::anyhow;
use clap::Parser;
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use rayon::iter::ParallelIterator;
use rayon::prelude::IntoParallelRefIterator;

#[derive(Parser)]
#[clap(author, version, about)]
struct Cli {
    /// The path to the Seatbelt compiler executable.
    #[clap(short, long, value_parser, default_value = "./Seatbelt")]
    seatbelt_path: PathBuf,

    /// The path to the Backseater virtual machine executable.
    #[clap(short, long, value_parser, default_value = "./backseat_safe_system_2k")]
    backseater_path: PathBuf,

    /// The path to the standard library for the Backseat language. The path must specify the
    /// parent directory of the std-folder.
    #[clap(short, long, value_parser, default_value = ".")]
    lib_path: PathBuf,

    /// The path of the Backseat source files to test. The source files have to start with
    /// 'test_' and end with '.bs' to be tested.
    #[clap(short, long, value_parser, default_value = ".")]
    tests_path: PathBuf,
}

#[derive(Debug, PartialEq)]
enum TestOutcome {
    Finished,
    Aborted { error_messages: Vec<String> },
}

struct TestResult {
    filename: String,
    kind: TestResultKind,
}

#[derive(Debug, PartialEq)]
enum TestResultKind {
    Success,
    Failure(String),
}

fn main() -> Result<(), Box<dyn Error>> {
    println!("test runner started");
    let cli = Cli::parse();

    let globwalker = globwalk::GlobWalkerBuilder::new(cli.tests_path.as_path(), "test*.bs")
        .build()
        .expect("unable to create glob walker");

    let source_files: Vec<_> = globwalker.collect::<Result<_, _>>()?;

    let tests_run = AtomicUsize::new(0);
    let tests_failed = AtomicUsize::new(0);

    source_files.par_iter().map(|source_file| -> anyhow::Result<TestResult> {
        std::io::stdout().flush().expect("unable to flush stdout");
        let expected_outcome = determine_expected_outcome(source_file.path())?;
        let filename = source_file.path().display().to_string();

        let command_result = Command::new(cli.seatbelt_path.as_os_str())
            .arg(&source_file.path().as_os_str())
            .arg("--lib")
            .arg(cli.lib_path.as_os_str())
            .stderr(Stdio::piped())
            .output()?;
        match command_result.status.success() {
            true => {
                let compiler_output = command_result.stdout;
                let backseater_result = child_with_pipe_args(
                    &cli.backseater_path,
                    compiler_output,
                    ["run", "--exit-on-halt"],
                )?;
                match backseater_result.status.success() {
                    true => {
                        if let TestOutcome::Aborted { error_messages } = expected_outcome {
                            let mut error_message = "\ttest execution finished, but the following error messages were expected:".to_string();
                            for message in error_messages {
                                error_message += &format!("\t\t\"{}\"", message);
                            }
                            Ok(TestResult { filename, kind: TestResultKind::Failure(error_message) })
                        } else {
                            Ok(TestResult{ filename, kind: TestResultKind::Success})
                        }
                    }
                    false => {
                        if let TestOutcome::Aborted { ref error_messages } = expected_outcome {
                            match validate_error_messages(
                                &backseater_result,
                                error_messages,
                            ) {
                                Ok(_) => Ok(TestResult { filename, kind: TestResultKind::Success }),
                                Err(error) => Ok(TestResult { filename, kind: TestResultKind::Failure(error.to_string()) }),
                            }
                        } else {
                            Ok(TestResult{filename, kind: TestResultKind::Failure(String::from_utf8(backseater_result.stderr)?)})
                        }
                    }
                }
            }
            false => {
                if let TestOutcome::Aborted { ref error_messages } = expected_outcome {
                    match validate_error_messages(
                        &command_result,
                        error_messages,
                    ) {
                        Ok(_) => Ok(TestResult { filename, kind: TestResultKind::Success }),
                        Err(error) => Ok(TestResult { filename, kind: TestResultKind::Failure(error.to_string()) }),
                    }
                } else {
                    Ok(TestResult{filename, kind: TestResultKind::Failure(String::from_utf8(command_result.stderr)?)})
                }
            }
        }
    }).for_each(|result| {
        match result {
            Ok(result) => {
                tests_run.fetch_add(1, Ordering::SeqCst);

                match result.kind {
                    TestResultKind::Success => {
                        print_success(&result.filename);
                    },
                    TestResultKind::Failure(error_message) => {
                        print_fail(&result.filename, &error_message);
                        tests_failed.fetch_add(1, Ordering::SeqCst);
                    },
                }
            },
            Err(_) => panic!(),
        }
    });

    let tests_run = tests_run.load(Ordering::Relaxed);
    let tests_failed = tests_failed.load(Ordering::Relaxed);

    let message = format!(
        "Tests run: {}, Tests successful: {}, Tests failed: {}\n",
        tests_run,
        tests_run - tests_failed,
        tests_failed
    );
    execute!(
        stdout(),
        SetForegroundColor(if tests_failed == 0 {
            Color::DarkGreen
        } else {
            Color::DarkRed
        }),
        Print(message),
        ResetColor
    )
    .expect("unable to print output");
    if tests_failed == 0 {
        Ok(())
    } else {
        Err("not all tests succeeded".into())
    }
}

fn print_success(filename: &str) {
    execute!(
        stdout(),
        Print(format!("test {filename} ... ")),
        SetForegroundColor(Color::DarkGreen),
        Print("OK\n"),
        ResetColor
    )
    .expect("unable to print output");
}

fn print_fail(filename: &str, error_message: &str) {
    execute!(
        stdout(),
        Print(format!("test {filename} ... ")),
        SetForegroundColor(Color::DarkRed),
        Print("FAILED\n"),
        ResetColor,
        Print(error_message)
    )
    .expect("unable to print output");
}

fn validate_error_messages(
    command_result: &std::process::Output,
    error_messages: &[String],
) -> anyhow::Result<()> {
    let stderr_string = String::from_utf8_lossy(&command_result.stderr);
    if error_messages
        .iter()
        .all(|message| stderr_string.contains(message))
    {
        Ok(())
    } else {
        let mut error_message = format!(
            "\ttest aborted as expected, but with wrong error message:\n\texpected: \"{}\"",
            error_messages[0]
        );
        for message in &error_messages[1..] {
            error_message += &format!("\t          and \"{}\"", &message);
        }
        error_message += &format!("\t     got: \"{}\"", stderr_string.trim());
        Err(anyhow!(error_message))
    }
}

fn determine_expected_outcome(source_file: &Path) -> anyhow::Result<TestOutcome> {
    let input_file = std::fs::read_to_string(source_file.as_os_str())?;
    let first_line = input_file.split('\n').next().unwrap().trim();
    if first_line.starts_with("//") {
        let test_runner_command = first_line.strip_prefix("//").unwrap().trim();
        let mut parts = test_runner_command.split('=');
        if let Some(lhs) = parts.next() {
            if let Some(rhs) = parts.next() {
                if lhs.trim() == "fails_with" {
                    let messages = rhs.trim().split(',');
                    let mut message_vector = Vec::new();
                    for message in messages {
                        let message = message.trim();
                        let message = message
                            .strip_prefix('"')
                            .ok_or_else(|| {
                                anyhow!("\" prefix not found in {}", source_file.display())
                            })?
                            .strip_suffix('"')
                            .ok_or_else(|| {
                                anyhow!("\" suffix not found in {}", source_file.display())
                            })?;
                        message_vector.push(String::from(message));
                    }
                    return Ok(TestOutcome::Aborted {
                        error_messages: message_vector,
                    });
                }
            }
        }
    }
    Ok(TestOutcome::Finished)
}

fn child_with_pipe_args<S, I>(
    path_of_executable: &Path,
    compiler_output: Vec<u8>,
    args: I,
) -> anyhow::Result<std::process::Output>
where
    S: AsRef<OsStr>,
    I: IntoIterator<Item = S>,
{
    let child = Command::new(path_of_executable.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(args)
        .spawn()?;
    spawn_child(child, compiler_output)
}

fn spawn_child(
    mut child: std::process::Child,
    compiler_output: Vec<u8>,
) -> anyhow::Result<std::process::Output> {
    let mut stdin = child.stdin.take().expect("Failed to open stdin");
    std::thread::spawn(move || {
        stdin
            .write_all(&compiler_output)
            .expect("Failed to write to stdin");
    });
    Ok(child.wait_with_output()?)
}
