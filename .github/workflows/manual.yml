name: Manual Testing Docs

on:
  pull_request:
  push:
    branches: [main]

jobs:
  check-generated:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v6

      - name: Check generated files are up to date
        run: scripts/gen-manual-testing.sh --check
