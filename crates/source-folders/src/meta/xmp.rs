//! XMP packets: reading the fields the app knows, and writing them into a packet (a new
//! one, or another program's sidecar, keeping everything else it holds).

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::ops::Range;

use c1model::{ColorLabel, GeoOrigin, GeoPoint, Iptc, KeywordPath, Photo, Place};
use quick_xml::NsReader;
use quick_xml::escape::{escape, unescape};
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::{PrefixDeclaration, QName, ResolveResult};

use super::embed;

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const NS_XMP: &str = "http://ns.adobe.com/xap/1.0/";
const NS_DC: &str = "http://purl.org/dc/elements/1.1/";
const NS_LR: &str = "http://ns.adobe.com/lightroom/1.0/";
const NS_PS: &str = "http://ns.adobe.com/photoshop/1.0/";
const NS_IPTC: &str = "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/";
const NS_EXIF: &str = "http://ns.adobe.com/exif/1.0/";

/// The header of the JPEG APP1 segment that holds the main XMP packet.
pub(crate) const JPEG_HEADER: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";
/// The TIFF tag that holds an XMP packet.
pub(crate) const TIFF_TAG: u16 = 700;
/// The largest packet read from a TIFF.
const MAX_TIFF_PACKET: u32 = 16 << 20;

/// What a metadata layer (XMP, IPTC) says about a photo. Empty where it says nothing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XmpFields {
    pub rating: Option<u8>,
    pub color: Option<ColorLabel>,
    pub keywords: Vec<KeywordPath>,
    pub iptc: Iptc,
    pub place: Place,
    pub geo: Option<GeoPoint>,
}

/// A property this module reads and writes.
struct Prop {
    ns: &'static str,
    /// The prefix used when the packet has no other for `ns`.
    prefix: &'static str,
    local: &'static str,
}

const RATING: Prop = Prop { ns: NS_XMP, prefix: "xmp", local: "Rating" };
const LABEL: Prop = Prop { ns: NS_XMP, prefix: "xmp", local: "Label" };
const SUBJECT: Prop = Prop { ns: NS_DC, prefix: "dc", local: "subject" };
const HIERARCHICAL: Prop = Prop { ns: NS_LR, prefix: "lr", local: "hierarchicalSubject" };
const TITLE: Prop = Prop { ns: NS_DC, prefix: "dc", local: "title" };
const HEADLINE: Prop = Prop { ns: NS_PS, prefix: "photoshop", local: "Headline" };
const DESCRIPTION: Prop = Prop { ns: NS_DC, prefix: "dc", local: "description" };
const CREATOR: Prop = Prop { ns: NS_DC, prefix: "dc", local: "creator" };
const RIGHTS: Prop = Prop { ns: NS_DC, prefix: "dc", local: "rights" };
const JOB_ID: Prop = Prop { ns: NS_PS, prefix: "photoshop", local: "TransmissionReference" };
const INSTRUCTIONS: Prop = Prop { ns: NS_PS, prefix: "photoshop", local: "Instructions" };
const SOURCE: Prop = Prop { ns: NS_PS, prefix: "photoshop", local: "Source" };
const CITY: Prop = Prop { ns: NS_PS, prefix: "photoshop", local: "City" };
const STATE: Prop = Prop { ns: NS_PS, prefix: "photoshop", local: "State" };
const COUNTRY: Prop = Prop { ns: NS_PS, prefix: "photoshop", local: "Country" };
const COUNTRY_CODE: Prop = Prop { ns: NS_IPTC, prefix: "Iptc4xmpCore", local: "CountryCode" };
const LOCATION: Prop = Prop { ns: NS_IPTC, prefix: "Iptc4xmpCore", local: "Location" };
const GPS_LAT: Prop = Prop { ns: NS_EXIF, prefix: "exif", local: "GPSLatitude" };
const GPS_LON: Prop = Prop { ns: NS_EXIF, prefix: "exif", local: "GPSLongitude" };
const GPS_ALT: Prop = Prop { ns: NS_EXIF, prefix: "exif", local: "GPSAltitude" };
const GPS_ALT_REF: Prop = Prop { ns: NS_EXIF, prefix: "exif", local: "GPSAltitudeRef" };

/// Every property `render` owns: replaced or removed in a packet it edits.
const OWNED: &[&Prop] = &[
    &RATING,
    &LABEL,
    &SUBJECT,
    &HIERARCHICAL,
    &TITLE,
    &HEADLINE,
    &DESCRIPTION,
    &CREATOR,
    &RIGHTS,
    &JOB_ID,
    &INSTRUCTIONS,
    &SOURCE,
    &CITY,
    &STATE,
    &COUNTRY,
    &COUNTRY_CODE,
    &LOCATION,
    &GPS_LAT,
    &GPS_LON,
    &GPS_ALT,
    &GPS_ALT_REF,
];

// ---------------------------------------------------------------------------------------
// Reading

/// An XML element, with names resolved to namespaces.
#[derive(Default)]
struct Node {
    ns: String,
    local: String,
    /// `(namespace, local name, value)`, without namespace declarations.
    attrs: Vec<(String, String, String)>,
    kids: Vec<Node>,
    text: String,
}

impl Node {
    fn is(&self, ns: &str, local: &str) -> bool {
        self.ns == ns && self.local == local
    }
}

/// The element tree of `xml`, or what was read of it before an error, if the root closed.
fn parse_tree(xml: &str) -> Option<Node> {
    let mut reader = NsReader::from_str(xml.trim_start_matches('\u{feff}'));
    let mut open: Vec<Node> = Vec::new();
    let mut root = None;
    let mut close = |open: &mut Vec<Node>, node: Node| match open.last_mut() {
        Some(parent) => parent.kids.push(node),
        None => root = root.take().or(Some(node)),
    };
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => open.push(node_of(&reader, &e)),
            Ok(Event::Empty(e)) => close(&mut open, node_of(&reader, &e)),
            Ok(Event::End(_)) => {
                let node = open.pop()?;
                close(&mut open, node);
            }
            Ok(Event::Text(t)) => push_text(&mut open, &t.xml10_content()),
            Ok(Event::CData(c)) => push_text(&mut open, &c.xml10_content()),
            Ok(Event::GeneralRef(r)) => {
                let resolved = match r.resolve_char_ref() {
                    Ok(Some(c)) => Some(c),
                    _ => predefined_entity(&r.into_inner()),
                };
                if let Some(c) = resolved {
                    push_text(&mut open, c.encode_utf8(&mut [0; 4]));
                }
            }
            Ok(Event::Eof) | Err(_) => break,
            Ok(_) => {}
        }
    }
    root
}

fn push_text(open: &mut [Node], text: &str) {
    if let Some(top) = open.last_mut() {
        top.text.push_str(text);
    }
}

fn predefined_entity(name: &str) -> Option<char> {
    match name {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "apos" => Some('\''),
        "quot" => Some('"'),
        _ => None,
    }
}

/// The namespace of a resolution, empty if there is none.
fn namespace(result: ResolveResult<'_>) -> String {
    match result {
        ResolveResult::Bound(ns) => ns.into_inner().to_owned(),
        _ => String::new(),
    }
}

fn node_of<R>(reader: &NsReader<R>, e: &BytesStart) -> Node {
    let (ns, local) = reader.resolver().resolve_element(e.name());
    let mut node = Node { ns: namespace(ns), local: local.into_inner().to_owned(), ..Node::default() };
    for attr in e.attributes().flatten() {
        if attr.key.as_namespace_binding().is_some() {
            continue;
        }
        let (ns, local) = reader.resolver().resolve_attribute(attr.key);
        let value = unescape(&attr.value).map(|v| v.into_owned()).unwrap_or_else(|_| attr.value.to_string());
        node.attrs.push((namespace(ns), local.into_inner().to_owned(), value));
    }
    node
}

/// A property's value as the packet gives it.
enum Value {
    Text(String),
    /// An `rdf:Alt`, `Bag` or `Seq`: `(language, text)` of each `rdf:li`.
    List(Vec<(Option<String>, String)>),
}

/// The properties of the `rdf:Description`s of a packet, the first of each kept.
struct Props(HashMap<(String, String), Value>);

impl Props {
    fn of(root: &Node) -> Self {
        let mut map = HashMap::new();
        collect(root, &mut map);
        Self(map)
    }

    fn get(&self, prop: &Prop) -> Option<&Value> {
        self.0.get(&(prop.ns.to_owned(), prop.local.to_owned()))
    }

    /// A plain value, or the preferred entry of a language alternative.
    fn text(&self, prop: &Prop) -> Option<String> {
        let text = match self.get(prop)? {
            Value::Text(t) => t.as_str(),
            Value::List(items) => {
                let item = items.iter().find(|(lang, _)| lang.as_deref() == Some("x-default")).or(items.first())?;
                item.1.as_str()
            }
        };
        Some(text.trim().to_owned()).filter(|t| !t.is_empty())
    }

    /// The entries of a bag or sequence.
    fn list(&self, prop: &Prop) -> Vec<String> {
        let items = match self.get(prop) {
            Some(Value::Text(t)) => vec![t.trim().to_owned()],
            Some(Value::List(items)) => items.iter().map(|(_, t)| t.trim().to_owned()).collect(),
            None => Vec::new(),
        };
        items.into_iter().filter(|t| !t.is_empty()).collect()
    }
}

fn collect(node: &Node, map: &mut HashMap<(String, String), Value>) {
    if !node.is(RDF, "Description") {
        node.kids.iter().for_each(|kid| collect(kid, map));
        return;
    }
    for (ns, local, value) in &node.attrs {
        map.entry((ns.clone(), local.clone())).or_insert_with(|| Value::Text(value.clone()));
    }
    for kid in &node.kids {
        let container = kid.kids.iter().find(|k| k.ns == RDF && matches!(k.local.as_str(), "Alt" | "Bag" | "Seq"));
        let value = match container {
            Some(list) => Value::List(list.kids.iter().filter(|li| li.is(RDF, "li")).map(item).collect()),
            None if kid.kids.is_empty() => Value::Text(kid.text.clone()),
            None => continue,
        };
        map.entry((kid.ns.clone(), kid.local.clone())).or_insert(value);
    }
}

fn item(li: &Node) -> (Option<String>, String) {
    let lang = li.attrs.iter().find(|(ns, local, _)| ns == XML && local == "lang").map(|(_, _, v)| v.clone());
    (lang, li.text.clone())
}

/// The fields the app knows in an XMP packet or sidecar. A malformed one gives what could
/// be read, usually nothing.
pub fn parse(xml: &str) -> XmpFields {
    let Some(root) = parse_tree(xml) else { return XmpFields::default() };
    let props = Props::of(&root);

    let hierarchical = props.list(&HIERARCHICAL);
    let keywords = if hierarchical.is_empty() {
        props.list(&SUBJECT).into_iter().map(|k| KeywordPath(vec![k])).collect()
    } else {
        hierarchical.iter().map(|k| KeywordPath::from_pipe(k)).collect()
    };
    let creators = props.list(&CREATOR).join("; ");
    XmpFields {
        rating: props
            .text(&RATING)
            .and_then(|r| r.parse::<f64>().ok())
            .filter(|r| (1.0..=5.0).contains(r))
            .map(|r| r as u8),
        color: props.text(&LABEL).and_then(|l| ColorLabel::from_name(&l)),
        keywords,
        iptc: Iptc {
            title: props.text(&TITLE),
            headline: props.text(&HEADLINE),
            description: props.text(&DESCRIPTION),
            creator: Some(creators).filter(|c| !c.is_empty()),
            copyright: props.text(&RIGHTS),
            job_id: props.text(&JOB_ID),
            instructions: props.text(&INSTRUCTIONS),
            source: props.text(&SOURCE),
        },
        place: Place {
            city: props.text(&CITY),
            state: props.text(&STATE),
            country: props.text(&COUNTRY),
            country_code: props.text(&COUNTRY_CODE),
            location: props.text(&LOCATION),
        },
        geo: geo(&props),
    }
}

fn geo(props: &Props) -> Option<GeoPoint> {
    let lat = parse_coordinate(&props.text(&GPS_LAT)?)?;
    let lon = parse_coordinate(&props.text(&GPS_LON)?)?;
    if !(-90.0..=90.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) || (lat == 0.0 && lon == 0.0) {
        return None;
    }
    let below = props.text(&GPS_ALT_REF).is_some_and(|r| r.trim() == "1");
    let alt = props.text(&GPS_ALT).and_then(|a| parse_number(&a)).map(|a| if below { -a } else { a });
    Some(GeoPoint { lat, lon, alt, origin: GeoOrigin::Exif })
}

/// `123/1` or `123.4`.
fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim();
    let value = match text.split_once('/') {
        Some((num, denom)) => num.trim().parse::<f64>().ok()? / denom.trim().parse::<f64>().ok()?,
        None => text.parse().ok()?,
    };
    value.is_finite().then_some(value)
}

/// An XMP coordinate: `50,2.2566833N`, `50,2,15.6N`, or decimal degrees with an optional
/// reference letter.
fn parse_coordinate(text: &str) -> Option<f64> {
    let text = text.trim();
    let (body, reference) = match text.chars().last()?.to_ascii_uppercase() {
        'N' | 'E' => (&text[..text.len() - 1], 1.0),
        'S' | 'W' => (&text[..text.len() - 1], -1.0),
        _ => (text, 1.0),
    };
    let parts: Vec<f64> = body.split(',').map(|p| p.trim().parse().ok()).collect::<Option<_>>()?;
    let (degrees, rest) = parts.split_first()?;
    let extra = match rest {
        [] => 0.0,
        [minutes] => minutes / 60.0,
        [minutes, seconds] => minutes / 60.0 + seconds / 3600.0,
        _ => return None,
    };
    let sign = if body.trim_start().starts_with('-') { -1.0 } else { 1.0 };
    let value = sign * (degrees.abs() + extra) * reference;
    value.is_finite().then_some(value)
}

// ---------------------------------------------------------------------------------------
// Finding a packet in a file

/// The XMP packet of a JPEG, from its APP1 segment. Extended XMP is ignored.
pub fn packet_from_jpeg(bytes: &[u8]) -> Option<String> {
    let (header, _) = embed::jpeg_header(bytes)?;
    let payload = header.iter().find(|s| s.marker == 0xE1 && bytes[s.payload.clone()].starts_with(JPEG_HEADER))?;
    Some(packet_text(&bytes[payload.payload.start + JPEG_HEADER.len()..payload.payload.end]))
}

/// A packet's text, without the padding (spaces, NULs) after it.
fn packet_text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim_end_matches(|c: char| c.is_whitespace() || c == '\0').to_owned()
}

/// The XMP packet of a TIFF or DNG (IFD0 tag 700), from its bytes.
pub fn packet_from_tiff(bytes: &[u8]) -> Option<String> {
    packet_from_tiff_reader(&mut std::io::Cursor::new(bytes))
}

/// [`packet_from_tiff`] for a file: only IFD0 and the packet are read.
pub fn packet_from_tiff_reader<R: Read + Seek>(reader: &mut R) -> Option<String> {
    let (offset, count) = tiff_packet_location(reader)?;
    reader.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = vec![0; count as usize];
    reader.read_exact(&mut bytes).ok()?;
    Some(packet_text(&bytes))
}

/// Where tag 700 of IFD0 keeps its bytes, and how many.
fn tiff_packet_location<R: Read + Seek>(reader: &mut R) -> Option<(u64, u32)> {
    reader.seek(SeekFrom::Start(0)).ok()?;
    let mut header = [0; 8];
    reader.read_exact(&mut header).ok()?;
    let (little, ifd0) = embed::tiff_header(&header)?;
    reader.seek(SeekFrom::Start(u64::from(ifd0))).ok()?;
    let mut count = [0; 2];
    reader.read_exact(&mut count).ok()?;
    let mut entries = vec![0; usize::from(embed::get_u16(&count, little)) * 12];
    reader.read_exact(&mut entries).ok()?;
    let entry = entries.chunks_exact(12).find(|e| embed::get_u16(e, little) == TIFF_TAG)?;
    let (kind, count) = (embed::get_u16(&entry[2..], little), embed::get_u32(&entry[4..], little));
    if !matches!(kind, 1 | 2 | 7) || !(5..=MAX_TIFF_PACKET).contains(&count) {
        // shorter than five bytes it would sit in the entry, and is no packet
        return None;
    }
    Some((u64::from(embed::get_u32(&entry[8..], little)), count))
}

// ---------------------------------------------------------------------------------------
// Writing

/// A property to write.
struct Out {
    prop: &'static Prop,
    shape: Shape,
    values: Vec<String>,
}

#[derive(Clone, Copy)]
enum Shape {
    Simple,
    /// `rdf:Alt` of one `x-default` entry.
    Alt,
    Bag,
    Seq,
}

/// The properties `photo` has values for.
fn outputs(photo: &Photo) -> Vec<Out> {
    let mut out = Vec::new();
    let mut add = |prop: &'static Prop, shape: Shape, values: Vec<String>| {
        let values: Vec<String> = values.into_iter().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).collect();
        if !values.is_empty() {
            out.push(Out { prop, shape, values });
        }
    };
    let one = |v: &Option<String>| v.iter().cloned().collect::<Vec<_>>();

    add(
        &RATING,
        Shape::Simple,
        photo.rating.filter(|r| (1..=5).contains(r)).map(|r| r.to_string()).into_iter().collect(),
    );
    add(&LABEL, Shape::Simple, photo.color.map(|c| capitalised(c.name())).into_iter().collect());

    let mut leaves: Vec<String> = Vec::new();
    for leaf in photo.keywords.iter().filter_map(|k| k.0.last()) {
        if !leaves.contains(leaf) {
            leaves.push(leaf.clone());
        }
    }
    let mut paths: Vec<String> = Vec::new();
    for path in photo.keywords.iter().map(KeywordPath::to_pipe) {
        if !paths.contains(&path) {
            paths.push(path);
        }
    }
    add(&SUBJECT, Shape::Bag, leaves);
    add(&HIERARCHICAL, Shape::Bag, paths);

    let iptc = &photo.iptc;
    add(&TITLE, Shape::Alt, one(&iptc.title));
    add(&HEADLINE, Shape::Simple, one(&iptc.headline));
    add(&DESCRIPTION, Shape::Alt, one(&iptc.description));
    add(&CREATOR, Shape::Seq, iptc.creator.iter().flat_map(|c| c.split("; ").map(str::to_owned)).collect());
    add(&RIGHTS, Shape::Alt, one(&iptc.copyright));
    add(&JOB_ID, Shape::Simple, one(&iptc.job_id));
    add(&INSTRUCTIONS, Shape::Simple, one(&iptc.instructions));
    add(&SOURCE, Shape::Simple, one(&iptc.source));

    let place = &photo.place;
    add(&CITY, Shape::Simple, one(&place.city));
    add(&STATE, Shape::Simple, one(&place.state));
    add(&COUNTRY, Shape::Simple, one(&place.country));
    add(&COUNTRY_CODE, Shape::Simple, one(&place.country_code));
    add(&LOCATION, Shape::Simple, one(&place.location));

    if let Some(geo) = photo.geo {
        add(&GPS_LAT, Shape::Simple, vec![format_coordinate(geo.lat, 'N', 'S')]);
        add(&GPS_LON, Shape::Simple, vec![format_coordinate(geo.lon, 'E', 'W')]);
        if let Some(alt) = geo.alt {
            add(&GPS_ALT, Shape::Simple, vec![format!("{}/100", (alt.abs() * 100.0).round())]);
            add(&GPS_ALT_REF, Shape::Simple, vec![if alt < 0.0 { "1" } else { "0" }.to_owned()]);
        }
    }
    out
}

fn capitalised(name: &str) -> String {
    let mut chars = name.chars();
    chars.next().map(|c| c.to_ascii_uppercase().to_string() + chars.as_str()).unwrap_or_default()
}

/// Degrees and decimal minutes with the hemisphere letter: `50,2.2566833N`.
fn format_coordinate(value: f64, positive: char, negative: char) -> String {
    let reference = if value < 0.0 { negative } else { positive };
    let value = value.abs();
    let mut degrees = value.floor();
    let mut minutes = ((value - degrees) * 60.0 * 1e6).round() / 1e6;
    if minutes >= 60.0 {
        degrees += 1.0;
        minutes = 0.0;
    }
    let minutes = format!("{minutes:.6}");
    let minutes = minutes.trim_end_matches('0').trim_end_matches('.');
    format!("{degrees},{minutes}{reference}")
}

/// The prefixes a packet uses for namespaces, and those still to be declared.
struct Prefixes {
    /// `(prefix, namespace)` already declared in the packet.
    bound: Vec<(String, String)>,
    /// Declarations to add.
    added: Vec<(String, String)>,
}

impl Prefixes {
    /// The prefix for `ns`: the one the packet has, else `preferred`, else `preferred` with a
    /// number, declared.
    fn of(&mut self, ns: &str, preferred: &str) -> String {
        let all = || self.bound.iter().chain(&self.added);
        if let Some((prefix, _)) = all().find(|(_, n)| n == ns) {
            return prefix.clone();
        }
        let taken = |p: &str| all().any(|(prefix, _)| prefix == p);
        let prefix = std::iter::once(preferred.to_owned())
            .chain((2..).map(|n| format!("{preferred}{n}")))
            .find(|p| !taken(p))
            .unwrap_or_default();
        self.added.push((prefix.clone(), ns.to_owned()));
        prefix
    }
}

/// The XML for `outputs`, each element on a line of its own after `indent`.
fn property_xml(outputs: &[Out], prefixes: &mut Prefixes, indent: &str) -> String {
    let rdf = prefixes.of(RDF, "rdf");
    let mut xml = String::new();
    for out in outputs {
        let name = format!("{}:{}", prefixes.of(out.prop.ns, out.prop.prefix), out.prop.local);
        let item = |text: &str, attr: &str| format!("\n{indent}  <{rdf}:li{attr}>{}</{rdf}:li>", escape(text));
        xml.push_str(&format!("\n{indent}"));
        match out.shape {
            Shape::Simple => xml.push_str(&format!("<{name}>{}</{name}>", escape(&out.values[0]))),
            shape => {
                let kind = match shape {
                    Shape::Alt => "Alt",
                    Shape::Seq => "Seq",
                    _ => "Bag",
                };
                let attr = if matches!(shape, Shape::Alt) { " xml:lang=\"x-default\"" } else { "" };
                xml.push_str(&format!("<{name}>\n{indent} <{rdf}:{kind}>"));
                for value in &out.values {
                    xml.push_str(&item(value, attr));
                }
                xml.push_str(&format!("\n{indent} </{rdf}:{kind}>\n{indent}</{name}>"));
            }
        }
    }
    xml
}

/// The packet for `photo`'s fields, merged into `existing` (another program's sidecar, say)
/// when there is one; see [`try_render`]. An `existing` that cannot be edited is replaced
/// by a new packet, so use [`try_render`] where that would lose something.
pub fn render(photo: &Photo, existing: Option<&str>) -> String {
    existing.and_then(|xml| try_render(photo, xml).ok()).unwrap_or_else(|| fresh(photo))
}

/// A new packet holding `photo`'s fields.
fn fresh(photo: &Photo) -> String {
    let outputs = outputs(photo);
    let mut prefixes = Prefixes { bound: vec![("rdf".to_owned(), RDF.to_owned())], added: Vec::new() };
    let body = property_xml(&outputs, &mut prefixes, "   ");
    let declarations: String =
        prefixes.added.iter().map(|(p, ns)| format!("\n    xmlns:{p}=\"{}\"", escape(ns))).collect();
    format!(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n\
         <x:xmpmeta xmlns:x=\"adobe:ns:meta/\" x:xmptk=\"c1studio\">\n \
         <rdf:RDF xmlns:rdf=\"{RDF}\">\n  \
         <rdf:Description rdf:about=\"\"{declarations}>{body}\n  </rdf:Description>\n \
         </rdf:RDF>\n\
         </x:xmpmeta>\n\
         <?xpacket end=\"w\"?>"
    )
}

/// An attribute in the text of a start tag.
struct AttrSpan {
    /// From the whitespace before the name to the closing quote, as offsets in the tag text.
    span: Range<usize>,
    key: String,
}

/// Where the attributes of a start tag (`<name a="1" b='2'/>`) are, and where the tag's name
/// and attribute list end.
fn tag_attributes(tag: &str) -> (Vec<AttrSpan>, usize) {
    let bytes = tag.as_bytes();
    let is_space = |b: u8| b.is_ascii_whitespace();
    let mut at = 1;
    while at < bytes.len() && !is_space(bytes[at]) && !matches!(bytes[at], b'>' | b'/') {
        at += 1;
    }
    let mut attrs = Vec::new();
    loop {
        let start = at;
        while at < bytes.len() && is_space(bytes[at]) {
            at += 1;
        }
        let key_start = at;
        while at < bytes.len() && !is_space(bytes[at]) && !matches!(bytes[at], b'=' | b'>' | b'/') {
            at += 1;
        }
        if at == key_start || at >= bytes.len() {
            return (attrs, start);
        }
        let key = tag[key_start..at].to_owned();
        while at < bytes.len() && (is_space(bytes[at]) || bytes[at] == b'=') {
            at += 1;
        }
        let Some(&quote) = bytes.get(at).filter(|q| matches!(q, b'"' | b'\'')) else { return (attrs, start) };
        let Some(len) = tag[at + 1..].find(quote as char) else { return (attrs, start) };
        at += len + 2;
        attrs.push(AttrSpan { span: start..at, key });
    }
}

/// The rdf:Description being edited.
struct Target {
    /// Its start tag in the packet.
    tag: Range<usize>,
    /// Owned attributes in the tag (offsets within it).
    owned: Vec<Range<usize>>,
    /// Where the tag's name and attributes end (offset within it).
    attrs_end: usize,
    /// The tag is `<.../>`.
    empty: bool,
}

/// Edits `existing`: the properties of `photo` replace the ones this module owns in its first
/// `rdf:Description`, and everything else is kept byte for byte. Errors if `existing` is not
/// XML with an `rdf:RDF` in it.
pub fn try_render(photo: &Photo, existing: &str) -> Result<String, String> {
    let invalid = |e: quick_xml::Error| format!("not valid XMP: {e}");
    let mut reader = NsReader::from_str(existing);
    let mut bound: Vec<(String, String)> = Vec::new();
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut target: Option<Target> = None;
    // elements open before the current event; `children` is the depth of the Description's children
    let mut depth = 0usize;
    let mut children: Option<usize> = None;
    let mut done = false;
    let mut blank: Option<usize> = None;
    let mut indent: Option<String> = None;
    let mut insert_at = None;
    let mut rdf_end = None;
    let mut seen_rdf = false;

    loop {
        let before = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(invalid)?;
        let after = reader.buffer_position() as usize;
        let blank_before = blank.take();
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let is_empty = matches!(event, Event::Empty(_));
                let (ns, local) = reader.resolver().resolve_element(e.name());
                let (ns, local) = (namespace(ns), local.into_inner().to_owned());
                if !done {
                    bound.extend(declarations(e));
                }
                seen_rdf |= ns == RDF && local == "RDF";
                if target.is_none() && !done && ns == RDF && local == "Description" {
                    let (attrs, attrs_end) = tag_attributes(&existing[before..after]);
                    let owned = attrs
                        .into_iter()
                        .filter(|attr| {
                            let (ns, local) = reader.resolver().resolve_attribute(QName(&attr.key));
                            is_owned(&namespace(ns), local.into_inner())
                        })
                        .map(|attr| attr.span)
                        .collect();
                    target = Some(Target { tag: before..after, owned, attrs_end, empty: is_empty });
                    done = is_empty;
                    children = (!is_empty).then_some(depth + 1);
                } else if children == Some(depth) {
                    if indent.is_none() {
                        indent = blank_before.map(|b| existing[b..before].rsplit('\n').next().unwrap_or("").to_owned());
                    }
                    if is_owned(&ns, &local) {
                        let end = if is_empty {
                            after
                        } else {
                            reader.read_to_end(e.name()).map_err(invalid)?;
                            reader.buffer_position() as usize
                        };
                        edits.push((blank_before.unwrap_or(before)..end, String::new()));
                        continue;
                    }
                }
                depth += usize::from(!is_empty);
            }
            Event::End(ref e) => {
                depth = depth.saturating_sub(1);
                if children == Some(depth + 1) {
                    insert_at = Some(blank_before.unwrap_or(before));
                    children = None;
                    done = true;
                } else if target.is_none() {
                    let (ns, local) = reader.resolver().resolve_element(e.name());
                    if namespace(ns) == RDF && local.into_inner() == "RDF" {
                        rdf_end = Some(blank_before.unwrap_or(before));
                    }
                }
            }
            Event::Text(ref t) if t.xml10_content().trim().is_empty() => blank = Some(before),
            Event::Eof => break,
            _ => {}
        }
    }

    let outputs = outputs(photo);
    let mut prefixes = Prefixes { bound, added: Vec::new() };
    let body = property_xml(&outputs, &mut prefixes, &indent.unwrap_or_else(|| "   ".to_owned()));
    let declared: String = prefixes.added.iter().map(|(p, ns)| format!(" xmlns:{p}=\"{}\"", escape(ns))).collect();

    match target {
        Some(target) => {
            let tag = &existing[target.tag.clone()];
            let mut new_tag = String::new();
            let mut at = 0;
            for r in &target.owned {
                new_tag.push_str(&tag[at..r.start]);
                at = r.end;
            }
            new_tag.push_str(&tag[at..target.attrs_end]);
            new_tag.push_str(&declared);
            if target.empty {
                let name =
                    tag[1..].split(|c: char| c.is_whitespace() || c == '>' || c == '/').next().unwrap_or_default();
                new_tag.push_str(&format!(">{body}\n</{name}>"));
            } else {
                new_tag.push_str(&tag[target.attrs_end..]);
                let at = insert_at.unwrap_or(target.tag.end);
                edits.push((at..at, body));
            }
            edits.push((target.tag, new_tag));
        }
        None => {
            let (Some(at), true) = (rdf_end, seen_rdf) else { return Err("no rdf:RDF in the XMP".to_owned()) };
            let rdf = prefixes.of(RDF, "rdf");
            edits.push((
                at..at,
                format!("\n  <{rdf}:Description {rdf}:about=\"\"{declared}>{body}\n  </{rdf}:Description>"),
            ));
        }
    }

    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut xml = existing.to_owned();
    for (range, text) in edits {
        xml.replace_range(range, &text);
    }
    Ok(xml)
}

/// The namespace declarations of a start tag, as `(prefix, namespace)`.
fn declarations(e: &BytesStart) -> Vec<(String, String)> {
    e.attributes()
        .flatten()
        .filter_map(|attr| match attr.key.as_namespace_binding()? {
            PrefixDeclaration::Named(prefix) => {
                let uri = unescape(&attr.value).map(|v| v.into_owned()).unwrap_or_default();
                Some((prefix.to_owned(), uri))
            }
            PrefixDeclaration::Default => None,
        })
        .collect()
}

/// Whether `render` owns the property `ns`:`local`.
fn is_owned(ns: &str, local: &str) -> bool {
    OWNED.iter().any(|p| p.ns == ns && p.local == local)
}

#[cfg(test)]
pub(crate) mod tests {
    use c1model::{AssetId, CollectionId, EditInfo, Exif, FileInfo, PhotoId, SourceId};

    use super::*;

    pub(crate) fn photo() -> Photo {
        let source = SourceId("folders:test".to_owned());
        Photo {
            id: PhotoId::new(&source, "0123456789abcdef"),
            asset: AssetId("0123456789abcdef".to_owned()),
            variant_index: 0,
            file: FileInfo::default(),
            captured_at: None,
            imported_at: None,
            exif: Exif::default(),
            rating: None,
            color: None,
            keywords: Vec::new(),
            iptc: Iptc::default(),
            place: Place::default(),
            geo: None,
            collections: Vec::<CollectionId>::new(),
            edit: EditInfo::default(),
            trashed: false,
        }
    }

    pub(crate) fn full_photo() -> Photo {
        let mut photo = photo();
        photo.rating = Some(4);
        photo.color = Some(ColorLabel::Purple);
        photo.keywords = vec![KeywordPath::from_pipe("places|poland|Wrocław"), KeywordPath::from_pipe("river & <sun>")];
        photo.iptc = Iptc {
            title: Some("Dom nad rzeką".to_owned()),
            headline: Some("Headline".to_owned()),
            description: Some("A <b>caption</b> & more".to_owned()),
            creator: Some("Ann; Bob".to_owned()),
            copyright: Some("(c) Ann".to_owned()),
            job_id: Some("job-7".to_owned()),
            instructions: Some("Do not crop".to_owned()),
            source: Some("Archive".to_owned()),
        };
        photo.place = Place {
            city: Some("Wrocław".to_owned()),
            state: Some("Dolnośląskie".to_owned()),
            country: Some("Poland".to_owned()),
            country_code: Some("PL".to_owned()),
            location: Some("Rynek".to_owned()),
        };
        photo.geo = Some(GeoPoint { lat: 51.1078852, lon: -17.0384, alt: Some(-12.5), origin: GeoOrigin::Manual });
        photo
    }

    fn fields_of(photo: &Photo) -> XmpFields {
        XmpFields {
            rating: photo.rating,
            color: photo.color,
            keywords: photo.keywords.clone(),
            iptc: photo.iptc.clone(),
            place: photo.place.clone(),
            geo: photo.geo.map(|g| GeoPoint { origin: GeoOrigin::Exif, ..g }),
        }
    }

    /// A Lightroom-style sidecar: develop settings as attributes and elements, a rating, a label
    /// and keywords of its own.
    pub(crate) const LIGHTROOM: &str = r#"<?xpacket begin="﻿" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        ">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:lr="http://ns.adobe.com/lightroom/1.0/"
    xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"
    xmp:Rating="2"
    xmp:Label="Red"
    crs:Version="15.0"
    crs:ProcessVersion="15.4"
    crs:Exposure2012="+0.35"
    crs:HasSettings="True">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>old</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <lr:hierarchicalSubject>
    <rdf:Bag>
     <rdf:li>old</rdf:li>
    </rdf:Bag>
   </lr:hierarchicalSubject>
   <crs:ToneCurvePV2012>
    <rdf:Seq>
     <rdf:li>0, 0</rdf:li>
     <rdf:li>255, 255</rdf:li>
    </rdf:Seq>
   </crs:ToneCurvePV2012>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;

    #[test]
    fn round_trips_a_new_packet() {
        let photo = full_photo();
        let xml = render(&photo, None);
        assert!(xml.starts_with("<?xpacket begin="));
        assert_eq!(parse(&xml), fields_of(&photo));
        // the coordinates keep their precision
        let geo = parse(&xml).geo.unwrap();
        assert!((geo.lat - 51.1078852).abs() < 1e-7 && (geo.lon + 17.0384).abs() < 1e-7);
    }

    #[test]
    fn an_empty_photo_writes_no_properties() {
        let xml = render(&photo(), None);
        assert_eq!(parse(&xml), XmpFields::default());
        assert!(!xml.contains("<xmp:") && !xml.contains("<dc:"));
    }

    #[test]
    fn reads_attribute_and_element_forms() {
        let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
          <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/"
             xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/" xmlns:exif="http://ns.adobe.com/exif/1.0/"
             xmlns:Iptc4xmpCore="http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/"
             xmp:Rating="3" xmp:Label="blue" photoshop:City="Kraków" exif:GPSLatitude="50,2.2566833N"
             exif:GPSLongitude="19,56,15.6E" exif:GPSAltitude="123/1" exif:GPSAltitudeRef="0">
            <dc:title><rdf:Alt><rdf:li xml:lang="pl">Tytuł</rdf:li><rdf:li xml:lang="x-default">Title</rdf:li></rdf:Alt></dc:title>
            <dc:creator><rdf:Seq><rdf:li>Ann</rdf:li><rdf:li>Bob</rdf:li></rdf:Seq></dc:creator>
            <dc:subject><rdf:Bag><rdf:li>a</rdf:li><rdf:li>b</rdf:li></rdf:Bag></dc:subject>
            <photoshop:Country>Poland &amp; Co</photoshop:Country>
            <Iptc4xmpCore:CountryCode>PL</Iptc4xmpCore:CountryCode>
          </rdf:Description></rdf:RDF></x:xmpmeta>"#;
        let f = parse(xml);
        assert_eq!((f.rating, f.color), (Some(3), Some(ColorLabel::Blue)));
        assert_eq!(f.iptc.title.as_deref(), Some("Title"));
        assert_eq!(f.iptc.creator.as_deref(), Some("Ann; Bob"));
        assert_eq!(f.keywords, [KeywordPath(vec!["a".into()]), KeywordPath(vec!["b".into()])]);
        assert_eq!(f.place.city.as_deref(), Some("Kraków"));
        assert_eq!(f.place.country.as_deref(), Some("Poland & Co"));
        assert_eq!(f.place.country_code.as_deref(), Some("PL"));
        let geo = f.geo.unwrap();
        assert!((geo.lat - (50.0 + 2.2566833 / 60.0)).abs() < 1e-9);
        assert!((geo.lon - (19.0 + 56.0 / 60.0 + 15.6 / 3600.0)).abs() < 1e-9);
        assert_eq!(geo.alt, Some(123.0));
    }

    #[test]
    fn ratings_and_labels() {
        let rating = |r: &str| {
            parse(&format!(
                r#"<rdf:RDF xmlns:rdf="{RDF}"><rdf:Description xmlns:xmp="{NS_XMP}" xmp:Rating="{r}"/></rdf:RDF>"#
            ))
            .rating
        };
        assert_eq!(
            [rating("0"), rating("-1"), rating("1"), rating("5"), rating("6"), rating("x")],
            [None, None, Some(1), Some(5), None, None]
        );
        assert_eq!(ColorLabel::from_name("PURPLE"), Some(ColorLabel::Purple));
        assert_eq!(parse("not xml at all <"), XmpFields::default());
    }

    #[test]
    fn hierarchical_keywords_win() {
        let xml = render(&full_photo(), None);
        let keywords = parse(&xml).keywords;
        assert_eq!(keywords[0], KeywordPath(vec!["places".into(), "poland".into(), "Wrocław".into()]));
        assert!(xml.contains("<rdf:li>Wrocław</rdf:li>"), "dc:subject holds the leaf names");
    }

    #[test]
    fn merges_into_a_lightroom_sidecar() {
        let mut photo = full_photo();
        photo.rating = Some(5);
        photo.color = None; // the label goes away
        let xml = render(&photo, Some(LIGHTROOM));
        // well formed, and the fields are ours
        let f = parse(&xml);
        assert_eq!(f.rating, Some(5));
        assert_eq!(f.color, None);
        assert_eq!(f.keywords, fields_of(&photo).keywords);
        assert_eq!(f.iptc, photo.iptc);
        // everything else is untouched, byte for byte
        for kept in [
            "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta",
            "x:xmptk=\"Adobe XMP Core 7.0-c000 1.000000, 0000/00/00-00:00:00        \"",
            "    crs:Version=\"15.0\"\n    crs:ProcessVersion=\"15.4\"\n    crs:Exposure2012=\"+0.35\"\n    crs:HasSettings=\"True\" xmlns:photoshop=",
            "xmlns:crs=\"http://ns.adobe.com/camera-raw-settings/1.0/\"",
            "   <crs:ToneCurvePV2012>\n    <rdf:Seq>\n     <rdf:li>0, 0</rdf:li>\n     <rdf:li>255, 255</rdf:li>\n    </rdf:Seq>\n   </crs:ToneCurvePV2012>\n",
            "\n  </rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
        ] {
            assert!(xml.contains(kept), "lost {kept:?} in\n{xml}");
        }
        assert!(
            !xml.contains("xmp:Rating=\"2\"") && !xml.contains("xmp:Label") && !xml.contains("<rdf:li>old</rdf:li>")
        );
        // namespaces were added where they were missing
        assert!(xml.contains("xmlns:photoshop=") && xml.contains("xmlns:exif=") && xml.contains("xmlns:Iptc4xmpCore="));
        // and writing again changes nothing
        assert_eq!(render(&photo, Some(&xml)), xml);
    }

    #[test]
    fn removes_what_is_now_empty() {
        let xml = render(&full_photo(), Some(LIGHTROOM));
        let cleared = render(&photo(), Some(&xml));
        assert_eq!(parse(&cleared), XmpFields::default());
        for gone in ["Rating", "Label", "subject", "hierarchicalSubject", "dc:title", "photoshop:City", "GPSLatitude"] {
            assert!(!cleared.contains(gone), "{gone} still in\n{cleared}");
        }
        assert!(cleared.contains("crs:Exposure2012=\"+0.35\"") && cleared.contains("<crs:ToneCurvePV2012>"));
    }

    #[test]
    fn edits_an_empty_description_and_one_with_no_description() {
        let empty = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="{RDF}"><rdf:Description rdf:about="" xmlns:xmp="{NS_XMP}" xmp:Rating="1" xmlns:crs="c" crs:Version="1"/></rdf:RDF></x:xmpmeta>"#
        );
        let xml = render(&full_photo(), Some(&empty));
        assert_eq!(parse(&xml).iptc, full_photo().iptc);
        assert!(xml.contains("crs:Version=\"1\"") && !xml.contains("xmp:Rating=\"1\""));

        let none = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="{RDF}">
</rdf:RDF></x:xmpmeta>"#
        );
        let xml = render(&full_photo(), Some(&none));
        assert_eq!(parse(&xml), fields_of(&full_photo()));
    }

    #[test]
    fn follows_the_prefixes_of_the_packet() {
        let other = format!(
            r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="{RDF}"><rdf:Description rdf:about="" xmlns:basic="{NS_XMP}" basic:Rating="2"></rdf:Description></rdf:RDF></x:xmpmeta>"#
        );
        let xml = render(&full_photo(), Some(&other));
        assert!(xml.contains("<basic:Label>Purple</basic:Label>"), "{xml}");
        assert!(!xml.contains("basic:Rating=\"2\""));
        assert_eq!(parse(&xml).rating, Some(4));
    }

    #[test]
    fn an_unusable_sidecar_is_an_error() {
        assert!(try_render(&photo(), "<a><b></a>").is_err());
        assert!(try_render(&photo(), "plain text").is_err());
        // `render` still gives a packet
        assert!(render(&full_photo(), Some("plain text")).contains("<x:xmpmeta"));
    }

    #[test]
    fn damage_does_not_panic() {
        for cut in (0..LIGHTROOM.len()).filter(|&i| LIGHTROOM.is_char_boundary(i)) {
            let _ = parse(&LIGHTROOM[..cut]);
            let _ = try_render(&full_photo(), &LIGHTROOM[..cut]);
        }
    }

    #[test]
    fn coordinates() {
        assert_eq!(format_coordinate(50.0 + 2.2566833 / 60.0, 'N', 'S'), "50,2.256683N");
        assert_eq!(format_coordinate(-0.5, 'E', 'W'), "0,30W");
        assert_eq!(format_coordinate(10.999_999_999, 'N', 'S'), "11,0N");
        assert_eq!(parse_coordinate("-50.5"), Some(-50.5));
        assert_eq!(parse_coordinate("50.5S"), Some(-50.5));
        assert_eq!(parse_coordinate("12,30W"), Some(-12.5));
        assert_eq!(parse_coordinate("x"), None);
    }
}
