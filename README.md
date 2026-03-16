# ast_serialize - Python Parser and AST Serializer

This is a fast Python extension for parsing Python files and serializing the AST using 
the native binary format used by mypy. This will eventually replace the current mypy parser,
which uses the Python stdlib `ast` module for parsing.

**This is work in progress.**

## Development

Prerequisites:

- Recent Rust toolchain
- Python 3.10+ (3.13t or 3.14t for free-threaded builds)
- [maturin](https://github.com/PyO3/maturin): `pip install maturin`
- Checkout of [mypy repo](https://github.com/python/mypy) for testing

Development build (fast compilation, unoptimized):
```bash
maturin develop
```

Optimized development build:
```bash
maturin develop --release
```

## Testing

**Rust unit tests:**
```bash
cargo test
```

**Python integration tests:** 
Run end-to-end parser and serialization/deserialization tests using mypy's test suite in 
the new-parser branch:
```bash
cd ~/src/mypy  [or wherever you have mypy]
pytest mypy -k NativeParser
```

Add new test cases to `test-data/unit/native-parser.test` (in the mypy repository). See
the main parser test cases in `parse.test` for the expected output format.

**Note:** Run `maturin develop` before testing if you've modified Rust code.

Use `TEST_NATIVE_PARSER=1 pytest mypy/test/testcheck.py` to run mypy type checking 
tests using ast_serialize. Note that some tests are still skipped.

## Creating PRs

You can create PRs in this repository, and/or in mypy repo. 
Contributions are welcome! Run mypy tests (see above) to get ideas about bugs and missing 
functionality. If your contributions needs changes in both mypy and ast_serialize, please
mention this in the PR summary.

## Building Wheels

```bash
# Build all wheels (stable ABI + free-threaded)
./build_wheels.sh

# Or manually:
# Stable ABI wheel (Python 3.9+)
maturin build --release

# Free-threaded wheel (Python 3.13t)
maturin build --release --no-default-features --features free-threaded --interpreter python3.13t
```

If you see `Python 3.13t not found`, you'll need to install the free-threaded build of CPython 3.13
in PATH.

## Making a release

1. Bump the version number in `pyproject.toml` in this repository.
2. Update `test_ast_serialize.py` (optional but recommended if the release includes major features).
3. Commit and push (pushing directly to master is fine).
4. Wait until all [builds](https://github.com/mypyc/ast_serialize/actions) complete successfully
   (no release is triggered yet).
5. Once builds are complete, tag the release (`git tag vX.Y.Z`; `git push origin vX.Y.Z`).
6. Go to the ["Actions" tab](https://github.com/mypyc/ast_serialize/actions) and click "Build wheels"
   on the left.
7. Click "Run workflow" and pick the newly created tag from the drop-down list. This will build
   *and upload* the wheels.
8. After the workflow completes, verify that `pip install -U ast-serialize` installs the new version
   from PyPI using a compiled wheel.
9. Create a PR to update the `ast-serialize` version in `pyproject.toml` in the mypy repository.

The process should take about 15 minutes.

## Using Coding Agents

This project is designed to support coding agent assisted development (such as Claude Code, Codex
or OpenCode). Notes:
 * Ensure your coding agent has access to AGENTS.md (e.g. create a CLAUDE.md symbolic link).
 * For the best experience, clone this repository as `~/src/ast_serialize`, and clone mypy
   (with the new-parser branch checked out) as `~/src/mypy`.
 * Have your mypy virtualenv active when starting the coding agent, or place the virtualenv
   at `~/venv/mypy` (this is referred to in AGENTS.md).

## Acknowledgments

This is a wrapper around the [Ruff](https://github.com/astral-sh/ruff) parser. Credits to Ruff 
maintainers for developing a fast Python parser!

## License

MIT
