"""Build Lumo's app and installer assets from the supplied logo, unchanged.

Run: python tools/make_brand.py (requires Pillow).
Source artwork is in docs/brand/lumo; the ICO is copied byte-for-byte.
"""
from pathlib import Path
import shutil
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
BRAND = ROOT / 'docs' / 'brand'
SOURCE = BRAND / 'lumo'
ART = ROOT / 'installer' / 'art'
RES = ROOT / 'crates' / 'lumo' / 'res'
SMALL = (58, 77, 97, 116, 124, 143, 159)
WIZARD = ((202, 386), (336, 643), (430, 824))


def picture(width, height, logo, share=.76):
    canvas = Image.new('RGBA', (width, height), 'white')
    side = round(min(width, height) * share)
    mark = logo.resize((side, side), Image.Resampling.LANCZOS)
    canvas.alpha_composite(mark, ((width - side) // 2, (height - side) // 2))
    return canvas.convert('RGB')


def main():
    ART.mkdir(parents=True, exist_ok=True)
    logo = Image.open(SOURCE / 'logo.png').convert('RGBA')
    shutil.copyfile(SOURCE / 'logo.ico', RES / 'lumo.ico')
    shutil.copyfile(SOURCE / 'logo.svg', BRAND / 'lumo-logo.svg')
    shutil.copyfile(SOURCE / 'logo-512.png', BRAND / 'lumo-logo-512.png')
    for size in SMALL:
        logo.resize((size, size), Image.Resampling.LANCZOS).save(ART / f'wizard-small-{size}.png', optimize=True)
    for width, height in WIZARD:
        picture(width, height, logo).save(ART / f'wizard-{width}x{height}.png', optimize=True)
    picture(164, 314, logo).save(ART / 'nsis-welcome.bmp')
    header = Image.new('RGBA', (150, 57), 'white')
    header.alpha_composite(logo.resize((48, 48), Image.Resampling.LANCZOS), (96, 4))
    header.convert('RGB').save(ART / 'nsis-header.bmp')
    picture(1200, 630, logo, .72).save(BRAND / 'social-preview.png', optimize=True)
    print('Updated Lumo application icon, installer artwork and documentation logo.')


if __name__ == '__main__':
    main()
