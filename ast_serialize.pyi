from typing import TypedDict, type_check_only
from typing_extensions import NotRequired, TypeAlias

__all__ = ["parse"]

_TypeIgnores: TypeAlias = list[tuple[int, list[str]]]

@type_check_only
class _ParseError(TypedDict):
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
) -> tuple[bytes, list[_ParseError], _TypeIgnores, bytes, bool]:
    ...
