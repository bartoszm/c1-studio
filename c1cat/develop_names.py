"""ZVARIANTLAYER column names as Capture One shows them.

Only the columns worth naming are listed; anything else is reported under its raw
column name, so a Capture One update that adds a slider degrades to "unknown
adjustment" rather than being silently dropped.
"""

from __future__ import annotations

# column -> (label, unit, tool group)
NAMES: dict[str, tuple[str, str, str]] = {
    "ZEXPOSURE": ("Exposure", "EV", "Exposure"),
    "ZCONTRAST": ("Contrast", "", "Exposure"),
    "ZBRIGHTNESS": ("Brightness", "", "Exposure"),
    "ZSATURATION": ("Saturation", "", "Exposure"),
    "ZHIGHLIGHTRECOVERY": ("Highlight", "", "High Dynamic Range"),
    "ZSHADOWRECOVERY": ("Shadow", "", "High Dynamic Range"),
    "ZWHITERECOVERY": ("White", "", "High Dynamic Range"),
    "ZBLACKRECOVERY": ("Black", "", "High Dynamic Range"),
    "ZCLARITY": ("Clarity", "", "Clarity"),
    "ZCLARITYSTRUCTURE": ("Structure", "", "Clarity"),
    "ZCLARITYMETHOD": ("Clarity method", "", "Clarity"),
    "ZDEHAZEAMOUNT": ("Dehaze", "", "Clarity"),
    "ZWHITEBALANCE": ("White balance", "RGB multipliers", "White Balance"),
    "ZLEVELSHIGHLIGHT": ("Levels highlight", "", "Levels"),
    "ZLEVELSSHADOW": ("Levels shadow", "", "Levels"),
    "ZGRADATIONCURVE": ("Curve (luma)", "", "Curve"),
    "ZGRADATIONCURVERED": ("Curve (red)", "", "Curve"),
    "ZGRADATIONCURVEGREEN": ("Curve (green)", "", "Curve"),
    "ZGRADATIONCURVEBLUE": ("Curve (blue)", "", "Curve"),
    "ZFILMCURVE": ("Curve preset", "", "Base Characteristics"),
    "ZICCPROFILE": ("ICC profile", "", "Base Characteristics"),
    "ZCROP": ("Crop", "cx;cy;w;h", "Crop"),
    "ZCROPASPECTRATIONAME": ("Crop ratio", "", "Crop"),
    "ZROTATION": ("Rotation", "deg", "Rotation & Flip"),
    "ZFLIP": ("Flip", "", "Rotation & Flip"),
    "ZBWENABLED": ("Black & White", "on", "Black & White"),
    "ZBWRED": ("B&W red", "", "Black & White"),
    "ZBWGREEN": ("B&W green", "", "Black & White"),
    "ZBWBLUE": ("B&W blue", "", "Black & White"),
    "ZBWYELLOW": ("B&W yellow", "", "Black & White"),
    "ZBWCYAN": ("B&W cyan", "", "Black & White"),
    "ZBWMAGENTA": ("B&W magenta", "", "Black & White"),
    "ZCOLORCORRECTIONS": ("Colour editor", "", "Color Editor"),
    "ZCOLORBALANCE": ("Colour balance", "", "Color Balance"),
    "ZCOLORBALANCESHADOW": ("Colour balance (shadow)", "", "Color Balance"),
    "ZCOLORBALANCEMIDTONE": ("Colour balance (midtone)", "", "Color Balance"),
    "ZCOLORBALANCEHIGHLIGHT": ("Colour balance (highlight)", "", "Color Balance"),
    "ZUSMAMOUNT": ("Sharpening amount", "", "Sharpening"),
    "ZUSMRADIUS": ("Sharpening radius", "", "Sharpening"),
    "ZUSMTHRESHOLD": ("Sharpening threshold", "", "Sharpening"),
    "ZUSMHALOCONTROL": ("Halo suppression", "", "Sharpening"),
    "ZNOISEREDUCTIONAMOUNT": ("Noise reduction", "", "Noise Reduction"),
    "ZNOISEDETAILSAMOUNT": ("Noise detail", "", "Noise Reduction"),
    "ZAIDENOISEAMOUNT": ("AI denoise", "", "Noise Reduction"),
    "ZCNRAMOUNT": ("Colour noise", "", "Noise Reduction"),
    "ZSURFACESMOOTHNESSAMOUNT": ("Surface smoothness", "", "Noise Reduction"),
    "ZMOIRE": ("Moire", "", "Moire"),
    "ZFILMGRAINAMOUNT": ("Grain amount", "", "Film Grain"),
    "ZFILMGRAINGRANULARITY": ("Grain granularity", "", "Film Grain"),
    "ZFILMGRAINTYPE": ("Grain type", "", "Film Grain"),
    "ZLENSDISTORTION": ("Lens distortion", "", "Lens Correction"),
    "ZLENSVIGNETTING": ("Vignetting", "", "Lens Correction"),
    "ZLENSCHROMABRENABLED": ("Chromatic aberration", "on", "Lens Correction"),
    "ZLENSPURPLEFRINGINGAMOUNT": ("Purple fringing", "", "Lens Correction"),
    "ZLENSDIFFRACTIONCORRECTIONENABLED": ("Diffraction correction", "on", "Lens Correction"),
    "ZGEOMETRYKEYSTONETILTX": ("Keystone vertical", "", "Keystone"),
    "ZGEOMETRYKEYSTONETILTY": ("Keystone horizontal", "", "Keystone"),
    "ZGEOMETRYKEYSTONEAMOUNT": ("Keystone amount", "", "Keystone"),
    "ZSPOTS": ("Spot removal", "", "Spot Removal"),
    "ZRELATIVEDUST": ("Dust removal", "", "Spot Removal"),
    "ZOPACITY": ("Layer opacity", "", "Layer"),
    "ZSTYLEUUID": ("Style", "", "Styles"),
    "ZAICOLORGRADE": ("AI colour grade", "on", "Styles"),
    "ZRETOUCHINGSKINEVENINGAMOUNT": ("Skin evening", "", "Retouching"),
    "ZRETOUCHINGBLEMISHREMOVALAMOUNT": ("Blemish removal", "", "Retouching"),
}

# Columns that are structural rather than user-facing adjustments.
IGNORED = {
    "Z_PK", "Z_ENT", "ZVARIANT", "ZINDEX", "ZMETADATA", "ZNAME", "ZENABLED",
    "ZCANVASSIZE", "ZIMAGEDIMENSIONS", "ZMASKUUID", "ZSTYLESOURCE", "ZSTYLEMODIFIED",
    "ZCORRESPONDINGPOINTS", "ZLAYERSELECTPOINT", "ZLENSIDGUID", "ZLENSREVISIONIDGUID",
    "ZLENSLCCBITMAPHASH", "ZBRUSHSTYLEUUID", "ZBRUSHSTYLENAME", "ZCONNECTORSACTION",
}
IGNORED |= {c for c in NAMES if False}
IGNORED |= {f"ZBRUSH{s}" for s in ("SIZE", "HARDNESS", "OPACITY", "FLOW", "AIRBRUSH",
                                   "USEPENPRESSURE", "AUTOMASK")}
IGNORED |= {f"ZERASER{s}" for s in ("SIZE", "HARDNESS", "OPACITY", "FLOW", "AIRBRUSH",
                                    "USEPENPRESSURE", "AUTOMASK")}


def label(column: str) -> str:
    entry = NAMES.get(column)
    return entry[0] if entry else column


def group(column: str) -> str:
    entry = NAMES.get(column)
    return entry[1 + 1] if entry else "Other"


def unit(column: str) -> str:
    entry = NAMES.get(column)
    return entry[1] if entry else ""


MAX_VALUE_CHARS = 70


def format_value(column: str, value) -> str:
    if value is None or value == "":
        return "-"
    if column == "ZCROP":
        try:
            cx, cy, w, h = (float(x) for x in str(value).split(";"))
            return f"{w:.0f}x{h:.0f} at ({cx:.0f},{cy:.0f})"
        except ValueError:
            return str(value)
    if isinstance(value, float):
        if column == "ZEXPOSURE":
            return f"{value:+.2f} EV"
        if column == "ZROTATION":
            return f"{value:+.2f} deg"
        return f"{value:+g}" if value else "0"
    text = str(value)
    if len(text) > MAX_VALUE_CHARS:
        # Curves, colour-editor tables and mask blobs are hundreds of characters of
        # packed numbers; say that they are set rather than printing them.
        return f"(set, {len(text)} chars)"
    return text
