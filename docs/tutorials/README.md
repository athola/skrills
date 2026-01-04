# Tutorials

This directory contains tutorial documentation with accompanying GIF demos.

## Structure

```
docs/tutorials/
├── README.md           # This file
├── quickstart.md       # Getting started tutorial
└── ...

assets/
├── tapes/              # VHS tape files for terminal recordings
│   ├── quickstart.tape
│   └── *.manifest.yaml # Multi-component tutorial manifests
└── gifs/               # Generated GIF outputs
    └── quickstart.gif
```

## Creating Tutorials

1. Write the tape file in `assets/tapes/<name>.tape`
2. Run `vhs assets/tapes/<name>.tape` to generate the GIF
3. Write documentation in `docs/tutorials/<name>.md`
4. Reference the GIF: `![Demo](../../assets/gifs/<name>.gif)`

## VHS Tape Format

```tape
# Example tape file
Output assets/gifs/example.gif
Set FontSize 14
Set Width 800
Set Height 400

Type "skrills --help"
Enter
Sleep 2s
```

See [VHS documentation](https://github.com/charmbracelet/vhs) for full syntax.
