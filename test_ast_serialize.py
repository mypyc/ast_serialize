#!/usr/bin/env python3
"""Smoke test for ast_serialize extension."""
import tempfile
import ast_serialize

def test_parse_simple_file():
    """Test that we can parse a simple Python file."""
    # Create a temporary Python file
    with tempfile.NamedTemporaryFile(mode='w', suffix='.py', delete=False) as f:
        f.write("print('hello')\n")
        fname = f.name

    try:
        # Parse the file
        result = ast_serialize.parse(fname)

        # Verify we got bytes back
        assert isinstance(result[0], bytes), f"Expected bytes, got {type(result)}"
        assert len(result[0]) > 0, "Expected non-empty result"

        print(f"Successfully parsed file: {len(result[0])} bytes")
        return True
    except Exception as e:
        print(f"Test failed: {e}")
        return False
    finally:
        import os
        os.unlink(fname)

if __name__ == '__main__':
    success = test_parse_simple_file()
    exit(0 if success else 1)
