from typing import NotRequired, TypedDict

TypeIgnores = list[tuple[int, list[str]]]

class ParseError(TypedDict):
    line: int
    column: int
    message: str
    blocker: NotRequired[bool]
    code: NotRequired[str]

def parse(
    fnam: str,
    skip_function_bodies: bool = False,
    python_version: tuple[int, int] | None = None,
    platform: str | None = None,
    always_true: list[str] | None = None,
    always_false: list[str] | None = None,
) -> tuple[bytes, list[ParseError], TypeIgnores, bytes, bool]:
    ...
