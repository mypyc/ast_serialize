import shutil
import subprocess

import tomlkit

CRATES = [
    "ruff_cache",
    "ruff_python_ast",
    "ruff_python_trivia",
    "ruff_text_size",
    "ruff_annotate_snippets",
    "ruff_macros",
    "ruff_python_parser",
    "ruff_source_file",
]


with open("Cargo.toml") as f:
    local_cargo = tomlkit.load(f)

local_version = local_cargo["package"]["version"]

for crate in CRATES:
    shutil.rmtree(f"crates/{crate}")
    shutil.copytree(f"../ruff/crates/{crate}", f"crates/{crate}")

    with open(f"crates/{crate}/Cargo.toml") as f:
        crate_toml = tomlkit.load(f)

    crate_version = crate_toml["package"]["version"]
    crate_toml["package"]["version"] = f"{crate_version}-ast-serialize.{local_version}"

    with open(f"crates/{crate}/Cargo.toml", "w") as f:
        crate_toml = tomlkit.dump(crate_toml, f)

subprocess.run(["git", "add", "-A", "crates"])

with open("../ruff/Cargo.toml") as f:
    ruff_cargo = tomlkit.load(f)


local_deps = local_cargo["workspace"]["dependencies"]
ruff_deps = ruff_cargo["workspace"]["dependencies"]

for dep in local_deps.copy():
    if dep.startswith("ruff_"):
        continue
    if dep not in ruff_deps:
        del local_deps[dep]
    else:
        local_deps[dep] = ruff_deps[dep]

with open("Cargo.toml", "w") as f:
    tomlkit.dump(local_cargo, f)
