#! /bin/bash

set -euo pipefail

source venv/bin/activate
pyinstaller PhotoViewer.spec --noconfirm
