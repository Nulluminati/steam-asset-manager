"""Load a script in bin/ as a module, so the tests call its functions."""

import importlib.util
from pathlib import Path

BIN = Path(__file__).resolve().parent.parent
ROOT = BIN.parent


def load_script(name: str):
    spec = importlib.util.spec_from_file_location(name, BIN / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module
