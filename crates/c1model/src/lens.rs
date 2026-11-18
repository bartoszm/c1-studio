//! One name per lens. Cameras and catalogs name the same lens in several ways: with the
//! maker in front or not (`Fujifilm Fujinon XF23mmF2 R WR`, `XF23mmF2 R WR`), and a
//! compact's built-in lens by its focal length (`23 mm f/2`) or by the camera's own name
//! (`Ricoh GR IV`). The index lists and filters lenses by [`lens_name`], so each lens is
//! one entry; the photo keeps what its file says.

/// The lens's name as the index lists it, or `None` when the camera recorded none
/// (an empty value, or `-- mm f/--`).
///
/// - A maker in front of a Fujifilm lens goes: `Fujifilm Fujinon XF23mmF2 R WR` is
///   `XF23mmF2 R WR`.
/// - A compact's built-in lens is `<camera> built-in`, whether it was recorded by focal
///   length or by the camera's name: `X100VI built-in`, `GR IV built-in`.
/// - A focal length with an unknown aperture drops it: `50 mm f/--` is `50 mm`.
pub fn lens_name(make: Option<&str>, model: Option<&str>, raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    let focal = focal_only(raw);
    if raw.is_empty() || focal.is_some_and(|(mm, _)| mm == "--") {
        return None;
    }
    if let Some(model) = model.map(str::trim).filter(|m| !m.is_empty()) {
        let camera = without_make(make, model);
        let names_camera = [model, camera].iter().any(|m| raw.eq_ignore_ascii_case(m))
            || without_make(make, raw).eq_ignore_ascii_case(camera);
        if names_camera || (focal.is_some() && fixed_lens(camera)) {
            return Some(format!("{camera} built-in"));
        }
    }
    Some(lens_pick(raw))
}

/// A lens name as picked or searched for, tidied as [`lens_name`] tidies a name that
/// needs no camera: a filter saved with `Fujifilm Fujinon XF23mmF2 R WR` still finds
/// `XF23mmF2 R WR`, and one saved with `50 mm f/--` finds `50 mm`. A built-in lens's old
/// name (`23 mm f/2`, `Ricoh GR IV`) needs the camera, so it stays as it is.
pub fn lens_pick(raw: &str) -> String {
    let raw = raw.trim();
    if let Some((mm, "--")) = focal_only(raw)
        && mm != "--"
    {
        return format!("{mm} mm");
    }
    for prefix in ["Fujifilm Fujinon ", "Fujinon ", "Fujifilm "] {
        if let Some(rest) = strip_prefix_ignore_case(raw, prefix)
            && fujifilm_series(rest)
        {
            return rest.to_owned();
        }
    }
    raw.to_owned()
}

/// A Fujifilm lens's name with the maker its file may give in front, for a `lens:` search
/// by the longer name (`lens:fujinon`, `lens:fujifilm xf`): `XF23mmF2 R WR` is
/// `Fujifilm Fujinon XF23mmF2 R WR`. `None` for other lenses.
pub fn lens_with_maker(name: &str) -> Option<String> {
    (fujifilm_series(name) && !name.ends_with(" built-in")).then(|| format!("Fujifilm Fujinon {name}"))
}

/// A Fujifilm lens series name: `XF…`, `XC…`, `GF…`.
fn fujifilm_series(name: &str) -> bool {
    ["XF", "XC", "GF"].iter().any(|series| starts_with_ignore_case(name, series))
}

/// `<focal> mm` or `<focal> mm f/<aperture>`, either part possibly `--`: the focal length
/// and the aperture (`""` when not given).
fn focal_only(s: &str) -> Option<(&str, &str)> {
    let (mm, rest) = s.split_once(" mm")?;
    let number = |v: &str| v == "--" || (!v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'));
    if !number(mm) {
        return None;
    }
    let aperture = match rest.trim_start() {
        "" => "",
        r => r.strip_prefix("f/").filter(|a| number(a))?,
    };
    Some((mm, aperture))
}

/// The camera's model without its maker's first word in front: `RICOH GR IV` by `RICOH
/// IMAGING COMPANY, LTD.` is `GR IV`; `Canon EOS 6D` is `EOS 6D`.
fn without_make<'a>(make: Option<&str>, model: &'a str) -> &'a str {
    let word = make.and_then(|m| m.split([' ', ',']).next()).unwrap_or("");
    if word.is_empty() {
        return model;
    }
    match strip_prefix_ignore_case(model, word) {
        Some(rest) if rest.starts_with(' ') => rest.trim_start(),
        _ => model,
    }
}

/// Cameras whose lens cannot be changed.
fn fixed_lens(camera: &str) -> bool {
    let c = camera.to_ascii_uppercase();
    let c = c.trim_start_matches("LEICA ");
    c.starts_with("X100")
        || c == "X70"
        || c == "XF10"
        || c == "GR"
        || c.starts_with("GR ")
        || c.starts_with("GR DIGITAL")
        || c.starts_with("GR II")
        || ["Q", "Q2", "Q3", "Q2 MONOCHROM", "Q3 43"].contains(&c)
        || c.contains("RX100")
        || c.contains("RX1")
        || c.starts_with("COOLPIX")
        || c.starts_with("POWERSHOT G")
}

fn strip_prefix_ignore_case<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    starts_with_ignore_case(s, prefix).then(|| &s[prefix.len()..])
}

fn starts_with_ignore_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len() && s.is_char_boundary(prefix.len()) && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FUJI: Option<&str> = Some("FUJIFILM");
    const RICOH: Option<&str> = Some("RICOH IMAGING COMPANY, LTD.");
    const CANON: Option<&str> = Some("Canon");

    fn name(make: Option<&str>, model: &str, raw: &str) -> Option<String> {
        lens_name(make, Some(model), Some(raw))
    }

    #[test]
    fn a_fujifilm_lens_is_one_name_with_or_without_the_maker() {
        for raw in ["Fujifilm Fujinon XF23mmF2 R WR", "XF23mmF2 R WR", " XF23mmF2 R WR "] {
            assert_eq!(name(FUJI, "X-Pro2", raw).as_deref(), Some("XF23mmF2 R WR"), "{raw}");
        }
        assert_eq!(
            name(FUJI, "X-Pro2", "Fujifilm Fujinon XF16-55mmF2.8 R LM WR").as_deref(),
            Some("XF16-55mmF2.8 R LM WR")
        );
        assert_eq!(name(FUJI, "X-Pro2", "XF16-55mmF2.8 R LM WR").as_deref(), Some("XF16-55mmF2.8 R LM WR"));
        assert_eq!(name(FUJI, "X-Pro2", "Fujifilm Fujinon XF35mmF1.4 R").as_deref(), Some("XF35mmF1.4 R"));
        assert_eq!(name(FUJI, "X-Pro2", "Fujifilm Fujinon XF18mmF2 R").as_deref(), Some("XF18mmF2 R"));
        assert_eq!(name(FUJI, "X-Pro2", "Fujifilm Fujinon XF27mmF2.8").as_deref(), Some("XF27mmF2.8"));
    }

    #[test]
    fn other_makers_lenses_stay_as_they_are() {
        for raw in ["Sigma 50mm F1.4 DG HSM | A (Canon EF)", "Tokina AT-X M100 AF PRO D (Canon EF)", "Fujifilm lens"] {
            assert_eq!(name(CANON, "Canon EOS 6D", raw).as_deref(), Some(raw), "{raw}");
        }
    }

    #[test]
    fn a_built_in_lens_is_named_by_its_camera() {
        assert_eq!(name(FUJI, "X100VI", "23 mm f/2").as_deref(), Some("X100VI built-in"));
        assert_eq!(name(RICOH, "RICOH GR IV", "18.3 mm f/--").as_deref(), Some("GR IV built-in"));
        assert_eq!(name(RICOH, "RICOH GR IV", "Ricoh GR IV").as_deref(), Some("GR IV built-in"));
        assert_eq!(name(RICOH, "RICOH GR IIIx", "26.1 mm f/2.8").as_deref(), Some("GR IIIx built-in"));
        assert_eq!(name(Some("LEICA CAMERA AG"), "LEICA Q2", "28 mm f/1.7").as_deref(), Some("Q2 built-in"));
    }

    #[test]
    fn a_focal_length_on_an_interchangeable_lens_camera_stays_a_focal_length() {
        assert_eq!(name(CANON, "Canon EOS 6D", "50 mm f/--").as_deref(), Some("50 mm"));
        assert_eq!(name(CANON, "Canon EOS 6D", "50 mm").as_deref(), Some("50 mm"));
        assert_eq!(name(CANON, "Canon EOS 6D", "50 mm f/1.8").as_deref(), Some("50 mm f/1.8"));
        assert_eq!(lens_name(None, None, Some("50 mm f/--")).as_deref(), Some("50 mm"));
    }

    #[test]
    fn no_lens_recorded_is_none() {
        assert_eq!(name(FUJI, "X-Pro2", "-- mm f/--"), None);
        assert_eq!(name(FUJI, "X-Pro2", ""), None);
        assert_eq!(name(FUJI, "X-Pro2", "  "), None);
        assert_eq!(lens_name(FUJI, Some("X-Pro2"), None), None);
        assert_eq!(name(Some("Movie"), "Movie", ""), None);
    }

    #[test]
    fn a_pick_is_tidied_like_a_name() {
        assert_eq!(lens_pick("Fujifilm Fujinon XF23mmF2 R WR"), "XF23mmF2 R WR");
        assert_eq!(lens_pick("fujinon xf23"), "xf23");
        assert_eq!(lens_pick("Fujinon"), "Fujinon");
        assert_eq!(lens_pick("X100VI built-in"), "X100VI built-in");
        assert_eq!(lens_pick("50 mm f/--"), "50 mm");
        assert_eq!(lens_pick("-- mm f/--"), "-- mm f/--");
        assert_eq!(lens_pick("50 mm f/1.8"), "50 mm f/1.8");
    }

    #[test]
    fn a_fujifilm_lens_is_searched_with_its_maker_too() {
        assert_eq!(lens_with_maker("XF23mmF2 R WR").as_deref(), Some("Fujifilm Fujinon XF23mmF2 R WR"));
        assert_eq!(lens_with_maker("GF63mmF2.8 R WR").as_deref(), Some("Fujifilm Fujinon GF63mmF2.8 R WR"));
        assert_eq!(lens_with_maker("XF10 built-in"), None);
        assert_eq!(lens_with_maker("50 mm"), None);
    }

    #[test]
    fn the_maker_is_taken_off_the_model_only_as_a_whole_word() {
        assert_eq!(without_make(Some("RICOH IMAGING COMPANY, LTD."), "RICOH GR IV"), "GR IV");
        assert_eq!(without_make(Some("Canon"), "Canon EOS 6D"), "EOS 6D");
        assert_eq!(without_make(Some("FUJIFILM"), "X100VI"), "X100VI");
        assert_eq!(without_make(Some("Ricoh"), "RicohX"), "RicohX");
        assert_eq!(without_make(None, "X100VI"), "X100VI");
    }
}
