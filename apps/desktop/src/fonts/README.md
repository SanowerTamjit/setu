# Bundled fonts

`SymbolsNerdFontMono-Regular.woff2` is *Symbols Nerd Font Mono* from the
[Nerd Fonts](https://github.com/ryanoasis/nerd-fonts) 3.4.0 release
(`NerdFontsSymbolsOnly.zip`), converted from TTF to WOFF2 without other changes.
It holds only icons (no letters) and Setu uses it as a fallback for icon code
points, so prompt themes such as Powerlevel10k and Starship show their icons
with any terminal font. See `docs/terminal.md`.

Licence: Nerd Fonts is MIT (`LICENSE-NerdFonts.txt`). The glyphs come from
these icon sets, under their own licences, all of which allow redistribution:
Powerline symbols and Powerline Extra (MIT), Font Awesome (SIL OFL 1.1),
Material Design Icons (Apache-2.0), Octicons (MIT), Devicons (MIT),
Codicons (CC BY 4.0, Microsoft), Font Logos (Unlicense), Weather Icons
(SIL OFL 1.1), Pomicons (SIL OFL 1.1) and IEC Power Symbols (MIT), as listed in
the Nerd Fonts README: https://github.com/ryanoasis/nerd-fonts#license

To update: download `NerdFontsSymbolsOnly.zip` from a Nerd Fonts release and run
`python3 -c "from fontTools.ttLib import TTFont; f=TTFont('SymbolsNerdFontMono-Regular.ttf'); f.flavor='woff2'; f.save('SymbolsNerdFontMono-Regular.woff2')"`
(needs `pip install fonttools brotli`).
