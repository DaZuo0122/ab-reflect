use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

const EXPECTED_OUTPUT_PREFIX: &str = "# Expected output:";

#[test]
fn run_all_integration_tests() {
    let bin = env!("CARGO_BIN_EXE_abx");
    let programs_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/programs");

    let entries: Vec<_> = fs::read_dir(&programs_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.ends_with(".abx.txt"))
                .unwrap_or(false)
        })
        .collect();

    assert!(
        !entries.is_empty(),
        "no .abx.txt programs found in {}",
        programs_dir.display()
    );

    for entry in entries {
        let path = entry.path();
        let name = path.file_name().unwrap().to_str().unwrap();
        let stem = name.strip_suffix(".abx.txt").unwrap();

        let mut cmd = Command::new(bin);
        cmd.arg(&path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Add CLI args if present
        let args_path = programs_dir.join(format!("{stem}.args"));
        if args_path.exists() {
            let args_content = fs::read_to_string(&args_path).unwrap();
            for arg in args_content.split_whitespace() {
                cmd.arg(arg);
            }
        }

        // Set up stdin if present
        let in_path = programs_dir.join(format!("{stem}.in"));
        let has_stdin = in_path.exists();
        if has_stdin {
            cmd.stdin(Stdio::piped());
        }

        let mut child = cmd.spawn().unwrap_or_else(|e| {
            panic!("failed to spawn abx for {stem}: {e}")
        });

        if has_stdin {
            let input = fs::read_to_string(&in_path).unwrap();
            std::io::Write::write_all(
                child.stdin.take().as_mut().unwrap(),
                input.as_bytes(),
            )
            .unwrap();
        }

        let output = child.wait_with_output().unwrap();

        // Check stdout
        let out_path = programs_dir.join(format!("{stem}.out"));
        if out_path.exists() {
            let expected = fs::read_to_string(&out_path).unwrap();
            let actual = String::from_utf8_lossy(&output.stdout);
            assert_eq!(
                actual, expected,
                "stdout mismatch for {stem}\n  expected: {expected:?}\n  actual:   {actual:?}"
            );
        } else {
            let source = fs::read_to_string(&path).unwrap();
            if let Some(expected) = expected_output_from_source(&source) {
                let actual = String::from_utf8_lossy(&output.stdout);
                assert_eq!(
                    actual, expected,
                    "stdout mismatch for {stem}\n  expected: {expected:?}\n  actual:   {actual:?}"
                );
            }
        }

        // Check stderr
        let err_path = programs_dir.join(format!("{stem}.err"));
        if err_path.exists() {
            let expected = fs::read_to_string(&err_path).unwrap();
            let actual = String::from_utf8_lossy(&output.stderr);
            assert_eq!(
                actual, expected,
                "stderr mismatch for {stem}\n  expected: {expected:?}\n  actual:   {actual:?}"
            );
        }

        // Check exit code
        let exit_path = programs_dir.join(format!("{stem}.exit"));
        let expected_exit = if exit_path.exists() {
            fs::read_to_string(&exit_path)
                .unwrap()
                .trim()
                .parse::<i32>()
                .unwrap()
        } else {
            0
        };
        assert_eq!(
            output.status.code().unwrap_or(-1),
            expected_exit,
            "exit code mismatch for {stem}"
        );
    }
}

fn expected_output_from_source(source: &str) -> Option<String> {
    source.lines().find_map(|line| {
        line.strip_prefix(EXPECTED_OUTPUT_PREFIX)
            .map(|text| decode_expected_output(text.strip_prefix(' ').unwrap_or(text)))
    })
}

fn decode_expected_output(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();

    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }

        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }

    out
}
