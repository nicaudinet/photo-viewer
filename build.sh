#! /bin/bash

set -euo pipefail

PYTHON=python3.11

if command -v $PYTHON >/dev/null 2>&1; then
    echo "Found $PYTHON"
else
    echo "$PYTHON not found"
    exit 1
fi

if [ ! -d venv ]; then
    echo "venv not found. Building"
    $PYTHON -m venv venv
fi

source venv/bin/activate

$PYTHON -m pip install -r requirements.txt

OS=$(uname -s)

if [[ "$OS" == "Linux" ]]; then
    pyinstaller --onefile main.linux.py

elif [[ "$OS" == "Darwin" ]]; then
    SPEC="PhotoViewer.macos.spec"
    pyinstaller "PhotoViewer.macos.spec" --noconfirm

else
    echo "Unsupported OS: $OS"
    exit 1
fi

