"""Run the full serial application suite and require a complete, passing result."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

def accepts_suite(log, returncode):
    """Do not mistake an empty, filtered, truncated or failed run for a clean suite."""
    runs = re.findall(r"^running (\d+) tests?$", log, re.MULTILINE)
    results = re.findall(r"^test (\S+)(?: - should panic)? \.\.\. (ok|FAILED|ignored(?:, [^\r\n]*)?)$", log, re.MULTILINE)
    summaries = re.findall(
        r"^test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored; "
        r"(\d+) measured; (\d+) filtered out; finished in [^\r\n]+$", log, re.MULTILINE,
    )
    if returncode != 0 or len(runs) != 1 or len(summaries) != 1:
        return False
    status, passed, failed, ignored, measured, filtered = summaries[0]
    counts = tuple(map(int, (passed, failed, ignored, measured, filtered)))
    names = [name for name, _ in results]
    return (status == "ok" and counts[0] > 0 and counts[1] == counts[3] == counts[4] == 0
            and int(runs[0]) == len(results) == counts[0] + counts[2]
            and len(names) == len(set(names))
            and sum(result == "ok" for _, result in results) == counts[0]
            and sum(result.startswith("ignored") for _, result in results) == counts[2]
            and "error: could not compile" not in log)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    root = Path(__file__).resolve().parents[1]
    result = subprocess.run(
        ["cargo", "test", "--manifest-path", str(root / "app/Cargo.toml"),
         "--locked", "--bin", "recordbox", "--", "--test-threads=1"],
        cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
        encoding="utf-8", errors="replace", env={**os.environ, "CARGO_TERM_COLOR": "never"},
    )
    log = result.stdout
    print(log, end="")
    (args.output / "cargo-test.txt").write_text(log, encoding="utf-8")
    accepted = accepts_suite(log, result.returncode)
    message = ("Full serial suite passed with no failed or filtered tests. "
               "Explicitly ignored manual/fixture tests remain visible in cargo-test.txt.\n"
               if accepted else "The complete application suite did not pass. "
               "Review cargo-test.txt for failures, filtering or an incomplete run.\n")
    (args.output / "tests.md").write_text(message, encoding="utf-8")
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write("### Application tests\n\n" + message)
    print(message)
    return 0 if accepted else 1


if __name__ == "__main__":
    sys.exit(main())
