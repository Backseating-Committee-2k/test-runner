use std::error::Error;
use std::ffi::OsStr;
use std::io::{stdout, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::Parser;
use crossterm::execute;
use crossterm::style::{Color, Print, ResetColor, SetForegroundColor};
use globwalk::DirEntry;

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

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    let mut tests_run: usize = 0;
    let mut tests_failed: usize = 0;
    let globwalker = globwalk::GlobWalkerBuilder::new(cli.tests_path.as_path(), "test*.bs")
        .build()
        .expect("unable to create glob walker");
    for source_file in globwalker {
        let source_file = source_file?;
        print!("test {} ... ", source_file.path().display());
        std::io::stdout().flush().expect("unable to flush stdout");
        let expected_outcome = determine_expected_outcome(source_file.path())?;

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
                            print_fail();
                            println!("\ttest execution finished, but the following error messages were expected:");
                            for message in error_messages {
                                println!("\t\t\"{}\"", message);
                            }
                            tests_failed += 1;
                        } else {
                            print_success();
                        }
                    }
                    false => {
                        if let TestOutcome::Aborted { ref error_messages } = expected_outcome {
                            validate_error_messages(
                                &backseater_result,
                                error_messages,
                                &mut tests_failed,
                            );
                        } else {
                            tests_failed += 1;
                            print_error(backseater_result);
                        }
                    }
                }
            }
            false => {
                if let TestOutcome::Aborted { ref error_messages } = expected_outcome {
                    validate_error_messages(&command_result, error_messages, &mut tests_failed);
                } else {
                    print_error(command_result);
                    tests_failed += 1;
                }
            }
        }
        tests_run += 1;
    }
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

fn print_success() {
    execute!(
        stdout(),
        SetForegroundColor(Color::DarkGreen),
        Print("OK\n"),
        ResetColor
    )
    .expect("unable to print output");
}

fn print_fail() {
    execute!(
        stdout(),
        SetForegroundColor(Color::DarkRed),
        Print("FAILED\n"),
        ResetColor
    )
    .expect("unable to print output");
}

fn validate_error_messages(
    command_result: &std::process::Output,
    error_messages: &[String],
    tests_failed: &mut usize,
) {
    let stderr_string = String::from_utf8_lossy(&command_result.stderr);
    if error_messages
        .iter()
        .all(|message| stderr_string.contains(message))
    {
        print_success();
    } else {
        print_fail();
        println!("\ttest aborted as expected, but with wrong error message:");
        println!("\texpected: \"{}\"", error_messages[0]);
        for message in &error_messages[1..] {
            println!("\t          and \"{}\"", &message);
        }
        println!("\t     got: \"{}\"", stderr_string.trim());
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
                    let messages = rhs.trim().split(',');
                    let mut message_vector = Vec::new();
                    for message in messages {
                        let message = message.trim();
                        let message = message
                            .strip_prefix('"')
                            .ok_or_else(|| {
                                format!("\" prefix not found in {}", source_file.display())
                            })?
                            .strip_suffix('"')
                            .ok_or_else(|| {
                                format!("\" suffix not found in {}", source_file.display())
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

fn child_with_pipe(
    path_of_executable: &Path,
    compiler_output: Vec<u8>,
) -> Result<std::process::Output, Box<dyn Error>> {
    let child = Command::new(path_of_executable.as_os_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
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
        .stderr(Stdio::piped())
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

fn print_error(command_result: std::process::Output) {
    print_fail();
    let error_message = String::from_utf8_lossy(&command_result.stderr);
    let error_message = error_message.replace('\n', "\n\t");
    println!("\t{error_message}");
}
