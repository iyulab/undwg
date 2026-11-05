//! The header variables: the drawing-wide settings (units, extents, the
//! current layer and text style, the dimension defaults, the handles of the
//! tables and the model and paper space blocks).
//!
//! The section opens with a sentinel and a byte count and closes with a
//! 16-bit checksum and a second sentinel, like the class section. R2000 and
//! R2004 keep every value, handles included, in one bit stream. From R2010 the
//! text values sit in a *string stream* and the handles in a *handle stream*,
//! both after the main stream; a bit count at the start of the data says where
//! the handle stream begins (the string stream ends just before it, the same
//! way it does in an object).
//!
//! Nothing is guessed: the checksum must match, and the main stream, the string
//! stream and the handle stream must each end exactly where the format puts
//! the end (a handle stream may leave up to seven padding bits).

use std::fmt;

use crate::bits::{BitError, BitReader, EndOfData, TextValue};
use crate::checksum::crc16;
use crate::version::Version;

const OPENING_SENTINEL: [u8; 16] = [
    0xCF, 0x7B, 0x1F, 0x23, 0xFD, 0xDE, 0x38, 0xA9, 0x5F, 0x7C, 0x68, 0xB8, 0x4E, 0x6D, 0x33, 0x5F,
];
const CLOSING_SENTINEL: [u8; 16] = [
    0x30, 0x84, 0xE0, 0xDC, 0x02, 0x21, 0xC7, 0x56, 0xA0, 0x83, 0x97, 0x47, 0xB1, 0x92, 0xCC, 0xA0,
];

/// The most bits that may follow the last field of the last stream.
const MAX_PADDING_BITS: usize = 8;

/// The seed of the checksum that covers the byte count and the data.
const CRC_SEED: u16 = 0xC0C1;

/// Why the header variables could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum VariablesError {
    /// The section is not laid out the way its version documents.
    Unexpected {
        /// What was wrong.
        what: &'static str,
    },
    /// The checksum does not match.
    Checksum {
        /// The value computed from the bytes.
        computed: u16,
        /// The value the section stores.
        stored: u16,
    },
    /// A value could not be read from the bit stream.
    Bits(BitError),
}

impl fmt::Display for VariablesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VariablesError::Unexpected { what } => write!(
                f,
                "the header variables' {what} is not what the format documents"
            ),
            VariablesError::Checksum { computed, stored } => write!(
                f,
                "header variables checksum mismatch: computed {computed:#06x}, the file stores {stored:#06x}"
            ),
            VariablesError::Bits(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for VariablesError {}

impl From<BitError> for VariablesError {
    fn from(e: BitError) -> Self {
        VariablesError::Bits(e)
    }
}

impl From<EndOfData> for VariablesError {
    fn from(e: EndOfData) -> Self {
        VariablesError::Bits(BitError::End(e))
    }
}

/// A date and time as the header stores it: a Julian day and the milliseconds
/// into that day, or a span as days and milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    /// The Julian day (for a span, the days).
    pub days: i32,
    /// The milliseconds into the day.
    pub milliseconds: i32,
}

/// A colour as the header states it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeaderColor {
    /// The colour number the file states (256 by layer, 0 by block).
    pub number: i16,
    /// The packed colour long (R2004 and later): the way in the top byte, the
    /// value in the rest; `None` in R2000.
    pub packed: Option<u32>,
    /// The colour name, when the file gives one.
    pub name: Option<TextValue>,
    /// The colour book name, when the file gives one.
    pub book_name: Option<TextValue>,
}

/// The values the header keeps once for model space and once for paper space.
#[derive(Debug, Clone, PartialEq)]
pub struct SpaceVariables {
    /// `INSBASE`: the insertion base point.
    pub insertion_base: [f64; 3],
    /// `EXTMIN`: the lower corner of the drawing extents.
    pub extents_min: [f64; 3],
    /// `EXTMAX`: the upper corner of the drawing extents.
    pub extents_max: [f64; 3],
    /// `LIMMIN`: the lower limit.
    pub limits_min: [f64; 2],
    /// `LIMMAX`: the upper limit.
    pub limits_max: [f64; 2],
    /// `ELEVATION`: the current elevation.
    pub elevation: f64,
    /// `UCSORG`: the origin of the user coordinate system.
    pub ucs_origin: [f64; 3],
    /// `UCSXDIR`: its x axis.
    pub ucs_x_axis: [f64; 3],
    /// `UCSYDIR`: its y axis.
    pub ucs_y_axis: [f64; 3],
    /// `UCSNAME`: the handle of the named coordinate system (0 for none).
    pub ucs_name: u64,
    /// `UCSORTHOREF`: the handle of the coordinate system an orthographic one
    /// is relative to.
    pub ucs_ortho_ref: u64,
    /// `UCSORTHOVIEW`: which orthographic view the coordinate system is.
    pub ucs_ortho_view: i16,
    /// `UCSBASE`: the handle of the base coordinate system.
    pub ucs_base: u64,
    /// The origins of the six orthographic coordinate systems, in the order
    /// top, bottom, left, right, front, back.
    pub ucs_ortho_origins: [[f64; 3]; 6],
}

/// The dimension variables: the defaults new dimensions start from.
#[derive(Debug, Clone, PartialEq)]
pub struct DimensionVariables {
    /// `DIMPOST`: the text prefix and suffix.
    pub post: TextValue,
    /// `DIMAPOST`: the alternate-unit prefix and suffix.
    pub alt_post: TextValue,
    /// `DIMSCALE`.
    pub scale: f64,
    /// `DIMASZ`: the arrow size.
    pub arrow_size: f64,
    /// `DIMEXO`: the extension line offset.
    pub extension_offset: f64,
    /// `DIMDLI`: the baseline spacing.
    pub baseline_spacing: f64,
    /// `DIMEXE`: the extension line extension.
    pub extension_extend: f64,
    /// `DIMRND`: the rounding.
    pub rounding: f64,
    /// `DIMDLE`: the dimension line extension.
    pub line_extend: f64,
    /// `DIMTP`: the plus tolerance.
    pub tolerance_plus: f64,
    /// `DIMTM`: the minus tolerance.
    pub tolerance_minus: f64,
    /// `DIMFXL`: the fixed extension line length (R2007 and later).
    pub fixed_extension_length: Option<f64>,
    /// `DIMJOGANG`: the jog angle (R2007 and later).
    pub jog_angle: Option<f64>,
    /// `DIMTFILL`: the text background (R2007 and later).
    pub text_fill: Option<i16>,
    /// `DIMTFILLCLR`: the text background colour (R2007 and later).
    pub text_fill_color: Option<HeaderColor>,
    /// `DIMTOL`: tolerances are appended.
    pub tolerance: bool,
    /// `DIMLIM`: limits are generated.
    pub limits: bool,
    /// `DIMTIH`: text inside the extension lines is horizontal.
    pub text_inside_horizontal: bool,
    /// `DIMTOH`: text outside the extension lines is horizontal.
    pub text_outside_horizontal: bool,
    /// `DIMSE1`: the first extension line is suppressed.
    pub suppress_extension_1: bool,
    /// `DIMSE2`: the second extension line is suppressed.
    pub suppress_extension_2: bool,
    /// `DIMTAD`: the vertical text position.
    pub text_above: i16,
    /// `DIMZIN`: zero suppression.
    pub zero_suppression: i16,
    /// `DIMAZIN`: angular zero suppression.
    pub angular_zero_suppression: i16,
    /// `DIMARCSYM`: the arc symbol (R2007 and later).
    pub arc_symbol: Option<i16>,
    /// `DIMTXT`: the text height.
    pub text_height: f64,
    /// `DIMCEN`: the centre mark size.
    pub center_mark: f64,
    /// `DIMTSZ`: the tick size.
    pub tick_size: f64,
    /// `DIMALTF`: the alternate-unit scale.
    pub alt_scale: f64,
    /// `DIMLFAC`: the linear scale factor.
    pub linear_factor: f64,
    /// `DIMTVP`: the vertical text position.
    pub text_vertical_position: f64,
    /// `DIMTFAC`: the tolerance text height factor.
    pub tolerance_height_factor: f64,
    /// `DIMGAP`: the gap around the text.
    pub gap: f64,
    /// `DIMALTRND`: the alternate-unit rounding.
    pub alt_rounding: f64,
    /// `DIMALT`: alternate units are shown.
    pub alt_units: bool,
    /// `DIMALTD`: the alternate-unit decimal places.
    pub alt_decimals: i16,
    /// `DIMTOFL`: a line is drawn between extension lines outside them.
    pub line_when_outside: bool,
    /// `DIMSAH`: separate arrow blocks.
    pub separate_arrow_blocks: bool,
    /// `DIMTIX`: text is forced inside the extension lines.
    pub text_inside: bool,
    /// `DIMSOXD`: arrows are suppressed outside the extension lines.
    pub suppress_outside_arrows: bool,
    /// `DIMCLRD`: the dimension line colour.
    pub line_color: HeaderColor,
    /// `DIMCLRE`: the extension line colour.
    pub extension_color: HeaderColor,
    /// `DIMCLRT`: the text colour.
    pub text_color: HeaderColor,
    /// `DIMADEC`: the angular decimal places.
    pub angular_decimals: i16,
    /// `DIMDEC`: the decimal places.
    pub decimals: i16,
    /// `DIMTDEC`: the tolerance decimal places.
    pub tolerance_decimals: i16,
    /// `DIMALTU`: the alternate-unit format.
    pub alt_unit_format: i16,
    /// `DIMALTTD`: the alternate-unit tolerance decimal places.
    pub alt_tolerance_decimals: i16,
    /// `DIMAUNIT`: the angular unit format.
    pub angular_unit_format: i16,
    /// `DIMFRAC`: the fraction format.
    pub fraction_format: i16,
    /// `DIMLUNIT`: the linear unit format.
    pub linear_unit_format: i16,
    /// `DIMDSEP`: the decimal separator, as a character code.
    pub decimal_separator: i16,
    /// `DIMTMOVE`: how text moves.
    pub text_move: i16,
    /// `DIMJUST`: the horizontal text justification.
    pub justify: i16,
    /// `DIMSD1`: the first dimension line is suppressed.
    pub suppress_line_1: bool,
    /// `DIMSD2`: the second dimension line is suppressed.
    pub suppress_line_2: bool,
    /// `DIMTOLJ`: the tolerance vertical justification.
    pub tolerance_justify: i16,
    /// `DIMTZIN`: the tolerance zero suppression.
    pub tolerance_zero_suppression: i16,
    /// `DIMALTZ`: the alternate-unit zero suppression.
    pub alt_zero_suppression: i16,
    /// `DIMALTTZ`: the alternate-unit tolerance zero suppression.
    pub alt_tolerance_zero_suppression: i16,
    /// `DIMUPT`: the user positions the text.
    pub user_positioned_text: bool,
    /// `DIMATFIT`: how text and arrows fit.
    pub fit: i16,
    /// `DIMFXLON`: the fixed extension line length is on (R2007 and later).
    pub fixed_extension_on: Option<bool>,
    /// `DIMTXTDIRECTION`: the text reads right to left (R2010 and later).
    pub text_direction: Option<bool>,
    /// `DIMALTMZF` (R2010 and later).
    pub alt_multiplier_factor: Option<f64>,
    /// `DIMALTMZS` (R2010 and later).
    pub alt_multiplier_suffix: Option<TextValue>,
    /// `DIMMZF` (R2010 and later).
    pub multiplier_factor: Option<f64>,
    /// `DIMMZS` (R2010 and later).
    pub multiplier_suffix: Option<TextValue>,
    /// `DIMTXSTY`: the handle of the dimension text style.
    pub text_style: u64,
    /// `DIMLDRBLK`: the handle of the leader arrow block.
    pub leader_block: u64,
    /// `DIMBLK`: the handle of the arrow block.
    pub block: u64,
    /// `DIMBLK1`: the handle of the first arrow block.
    pub block_1: u64,
    /// `DIMBLK2`: the handle of the second arrow block.
    pub block_2: u64,
    /// `DIMLTYPE`: the handle of the dimension line type (R2007 and later).
    pub line_type: Option<u64>,
    /// `DIMLTEX1`: the first extension line type (R2007 and later).
    pub extension_line_type_1: Option<u64>,
    /// `DIMLTEX2`: the second extension line type (R2007 and later).
    pub extension_line_type_2: Option<u64>,
    /// `DIMLWD`: the dimension line weight.
    pub line_weight: i16,
    /// `DIMLWE`: the extension line weight.
    pub extension_line_weight: i16,
}

/// The handles of the table control objects and the named dictionaries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlHandles {
    /// The block table.
    pub block: u64,
    /// The layer table.
    pub layer: u64,
    /// The text style table.
    pub style: u64,
    /// The line type table.
    pub linetype: u64,
    /// The view table.
    pub view: u64,
    /// The coordinate system table.
    pub ucs: u64,
    /// The viewport table.
    pub vport: u64,
    /// The registered application table.
    pub appid: u64,
    /// The dimension style table.
    pub dimstyle: u64,
    /// The viewport entity header table (R2000 only).
    pub viewport_entity_header: Option<u64>,
    /// `ACAD_GROUP`.
    pub group_dictionary: u64,
    /// `ACAD_MLINESTYLE`.
    pub mline_style_dictionary: u64,
    /// The named objects dictionary (the root of all dictionaries).
    pub named_objects: u64,
    /// The layouts dictionary.
    pub layouts: u64,
    /// The plot settings dictionary.
    pub plot_settings: u64,
    /// The plot styles dictionary.
    pub plot_styles: u64,
    /// The materials dictionary (R2004 and later).
    pub materials: Option<u64>,
    /// The colours dictionary (R2004 and later).
    pub colors: Option<u64>,
    /// The visual styles dictionary (R2007 and later).
    pub visual_styles: Option<u64>,
    /// A handle the format does not name (R2013 and later).
    pub unnamed: Option<u64>,
}

/// The header variables of a drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct HeaderVariables {
    /// `REQUIREDVERSIONS` (R2013 and later).
    pub required_versions: Option<u64>,
    /// The handle of the current viewport entity header (before R2004).
    pub current_viewport: Option<u64>,
    /// `DIMASO`.
    pub dim_associative: bool,
    /// `DIMSHO`.
    pub dim_show_while_dragging: bool,
    /// `PLINEGEN`: line types are generated across the vertices of a polyline.
    pub polyline_generation: bool,
    /// `ORTHOMODE`.
    pub ortho_mode: bool,
    /// `REGENMODE`.
    pub regen_mode: bool,
    /// `FILLMODE`.
    pub fill_mode: bool,
    /// `QTEXTMODE`.
    pub quick_text_mode: bool,
    /// `PSLTSCALE`.
    pub paper_space_line_type_scale: bool,
    /// `LIMCHECK`.
    pub limits_check: bool,
    /// `USRTIMER`: the user timer is on.
    pub user_timer: bool,
    /// `SKPOLY`.
    pub sketch_polylines: bool,
    /// `ANGDIR`: angles count clockwise.
    pub angle_direction: bool,
    /// `SPLFRAME`.
    pub spline_frame: bool,
    /// `MIRRTEXT`.
    pub mirror_text: bool,
    /// `WORLDVIEW`.
    pub world_view: bool,
    /// `TILEMODE`: the model tab is current.
    pub tile_mode: bool,
    /// `PLIMCHECK`.
    pub paper_limits_check: bool,
    /// `VISRETAIN`.
    pub retain_xref_visibility: bool,
    /// `DISPSILH`.
    pub display_silhouettes: bool,
    /// `PELLIPSE`.
    pub polyline_ellipses: bool,
    /// `PROXYGRAPHICS`.
    pub proxy_graphics: i16,
    /// `TREEDEPTH`.
    pub tree_depth: i16,
    /// `LUNITS`: the linear unit format.
    pub linear_units: i16,
    /// `LUPREC`: the linear decimal places.
    pub linear_precision: i16,
    /// `AUNITS`: the angular unit format.
    pub angular_units: i16,
    /// `AUPREC`: the angular decimal places.
    pub angular_precision: i16,
    /// `ATTMODE`.
    pub attribute_mode: i16,
    /// `PDMODE`: the point display style.
    pub point_mode: i16,
    /// `USERI1` to `USERI5`.
    pub user_integers: [i16; 5],
    /// `SPLINESEGS`.
    pub spline_segments: i16,
    /// `SURFU`.
    pub surface_u: i16,
    /// `SURFV`.
    pub surface_v: i16,
    /// `SURFTYPE`.
    pub surface_type: i16,
    /// `SURFTAB1`.
    pub surface_tab_1: i16,
    /// `SURFTAB2`.
    pub surface_tab_2: i16,
    /// `SPLINETYPE`.
    pub spline_type: i16,
    /// `SHADEDGE`.
    pub shade_edge: i16,
    /// `SHADEDIF`.
    pub shade_diffuse: i16,
    /// `UNITMODE`.
    pub unit_mode: i16,
    /// `MAXACTVP`.
    pub max_active_viewports: i16,
    /// `ISOLINES`.
    pub iso_lines: i16,
    /// `CMLJUST`.
    pub mline_justification: i16,
    /// `TEXTQLTY`.
    pub text_quality: i16,
    /// `LTSCALE`.
    pub line_type_scale: f64,
    /// `TEXTSIZE`.
    pub text_size: f64,
    /// `TRACEWID`.
    pub trace_width: f64,
    /// `SKETCHINC`.
    pub sketch_increment: f64,
    /// `FILLETRAD`.
    pub fillet_radius: f64,
    /// `THICKNESS`.
    pub thickness: f64,
    /// `ANGBASE`.
    pub angle_base: f64,
    /// `PDSIZE`: the point display size.
    pub point_size: f64,
    /// `PLINEWID`.
    pub polyline_width: f64,
    /// `USERR1` to `USERR5`.
    pub user_reals: [f64; 5],
    /// `CHAMFERA` to `CHAMFERD`.
    pub chamfer: [f64; 4],
    /// `FACETRES`.
    pub facet_resolution: f64,
    /// `CMLSCALE`.
    pub mline_scale: f64,
    /// `CELTSCALE`.
    pub entity_line_type_scale: f64,
    /// `MENUNAME`.
    pub menu_name: TextValue,
    /// `TDCREATE`: when the drawing was created.
    pub created: Timestamp,
    /// `TDUPDATE`: when the drawing was last saved.
    pub updated: Timestamp,
    /// `TDINDWG`: the time spent in the drawing.
    pub time_in_drawing: Timestamp,
    /// `TDUSRTIMER`: the user timer.
    pub user_timer_value: Timestamp,
    /// `CECOLOR`: the current entity colour.
    pub current_color: HeaderColor,
    /// `HANDSEED`: the next handle to be given out.
    pub handle_seed: u64,
    /// `CLAYER`: the handle of the current layer.
    pub current_layer: u64,
    /// `TEXTSTYLE`: the handle of the current text style.
    pub current_text_style: u64,
    /// `CELTYPE`: the handle of the current line type.
    pub current_line_type: u64,
    /// `CMATERIAL`: the handle of the current material (R2007 and later).
    pub current_material: Option<u64>,
    /// `DIMSTYLE`: the handle of the current dimension style.
    pub current_dim_style: u64,
    /// `CMLSTYLE`: the handle of the current multiline style.
    pub current_mline_style: u64,
    /// `PSVPSCALE`: the paper space viewport scale.
    pub paper_viewport_scale: f64,
    /// The values of paper space.
    pub paper_space: SpaceVariables,
    /// The values of model space.
    pub model_space: SpaceVariables,
    /// The dimension variables.
    pub dimension: DimensionVariables,
    /// The control objects and the named dictionaries.
    pub controls: ControlHandles,
    /// `TSTACKALIGN`.
    pub stack_alignment: i16,
    /// `TSTACKSIZE`.
    pub stack_size: i16,
    /// `HYPERLINKBASE`.
    pub hyperlink_base: TextValue,
    /// `STYLESHEET`.
    pub style_sheet: TextValue,
    /// The packed flags: `CELWEIGHT` `0x1F`, `ENDCAPS` `0x60`, `JOINSTYLE`
    /// `0x180`, `LWDISPLAY` (set means off) `0x200`, `XEDIT` (set means off)
    /// `0x400`, `EXTNAMES` `0x800`, `PSTYLEMODE` `0x2000`, `OLESTARTUP` `0x4000`.
    pub flags: i32,
    /// `INSUNITS`: the units of insertion.
    pub insertion_units: i16,
    /// `CEPSNTYPE`: the plot style type of new entities.
    pub plot_style_type: i16,
    /// `CPSNID`: the handle of the plot style, when the type names one.
    pub plot_style_id: Option<u64>,
    /// `FINGERPRINTGUID`.
    pub fingerprint_guid: TextValue,
    /// `VERSIONGUID`.
    pub version_guid: TextValue,
    /// `SORTENTS` (R2004 and later).
    pub sort_entities: Option<u8>,
    /// `INDEXCTL` (R2004 and later).
    pub index_control: Option<u8>,
    /// `HIDETEXT` (R2004 and later).
    pub hide_text: Option<u8>,
    /// `XCLIPFRAME` (R2004 and later).
    pub xclip_frame: Option<u8>,
    /// `DIMASSOC` (R2004 and later).
    pub dim_association: Option<u8>,
    /// `HALOGAP` (R2004 and later).
    pub halo_gap: Option<u8>,
    /// `OBSCUREDCOLOR` (R2004 and later).
    pub obscured_color: Option<i16>,
    /// `INTERSECTIONCOLOR` (R2004 and later).
    pub intersection_color: Option<i16>,
    /// `OBSCUREDLTYPE` (R2004 and later).
    pub obscured_line_type: Option<u8>,
    /// `INTERSECTIONDISPLAY` (R2004 and later).
    pub intersection_display: Option<u8>,
    /// `PROJECTNAME` (R2004 and later).
    pub project_name: Option<TextValue>,
    /// The handle of the paper space block record.
    pub paper_space_block: u64,
    /// The handle of the model space block record.
    pub model_space_block: u64,
    /// The handle of the `ByLayer` line type.
    pub line_type_by_layer: u64,
    /// The handle of the `ByBlock` line type.
    pub line_type_by_block: u64,
    /// The handle of the `Continuous` line type.
    pub line_type_continuous: u64,
    /// The later settings (R2007 and later): camera, solids, loft, geographic
    /// location, visual styles.
    pub extended: Option<ExtendedVariables>,
}

/// The header variables that R2007 added.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtendedVariables {
    /// `CAMERADISPLAY`.
    pub camera_display: bool,
    /// `STEPSPERSEC`.
    pub steps_per_second: f64,
    /// `STEPSIZE`.
    pub step_size: f64,
    /// `3DDWFPREC`.
    pub dwf_precision: f64,
    /// `LENSLENGTH`.
    pub lens_length: f64,
    /// `CAMERAHEIGHT`.
    pub camera_height: f64,
    /// `SOLIDHIST`.
    pub solid_history: u8,
    /// `SHOWHIST`.
    pub show_history: u8,
    /// `PSOLWIDTH`.
    pub solid_width: f64,
    /// `PSOLHEIGHT`.
    pub solid_height: f64,
    /// `LOFTANG1`.
    pub loft_angle_1: f64,
    /// `LOFTANG2`.
    pub loft_angle_2: f64,
    /// `LOFTMAG1`.
    pub loft_magnitude_1: f64,
    /// `LOFTMAG2`.
    pub loft_magnitude_2: f64,
    /// `LOFTPARAM`.
    pub loft_parameters: i16,
    /// `LOFTNORMALS`.
    pub loft_normals: u8,
    /// `LATITUDE`.
    pub latitude: f64,
    /// `LONGITUDE`.
    pub longitude: f64,
    /// `NORTHDIRECTION`.
    pub north_direction: f64,
    /// `TIMEZONE`.
    pub time_zone: i32,
    /// `LIGHTGLYPHDISPLAY`.
    pub light_glyph_display: u8,
    /// `TILEMODELIGHTSYNCH`.
    pub tile_model_light_synch: u8,
    /// `DWFFRAME`.
    pub dwf_frame: u8,
    /// `DGNFRAME`.
    pub dgn_frame: u8,
    /// `INTERFERECOLOR`.
    pub interfere_color: HeaderColor,
    /// `INTERFEREOBJVS`.
    pub interfere_object_visual_style: u64,
    /// `INTERFEREVPVS`.
    pub interfere_viewport_visual_style: u64,
    /// `DRAGVS`.
    pub drag_visual_style: u64,
    /// `CSHADOW`.
    pub cast_shadows: u8,
}

/// The streams a header's values are read from.
struct Streams<'a> {
    main: BitReader<'a>,
    /// The string stream, from R2010; before that the text is in `main`.
    strings: Option<BitReader<'a>>,
    /// The handle stream, from R2010; before that the handles are in `main`.
    handles: Option<BitReader<'a>>,
}

impl Streams<'_> {
    fn text(&mut self) -> Result<TextValue, BitError> {
        match &mut self.strings {
            Some(s) => s.text(true),
            None => self.main.text(false),
        }
    }

    /// A handle that names a table, object or style by its absolute value (0
    /// names nothing). The header has no holder, so a relative handle is wrong.
    fn handle(&mut self) -> Result<u64, VariablesError> {
        let r = match &mut self.handles {
            Some(h) => h,
            None => &mut self.main,
        };
        let h = r.handle()?;
        if h.code > 5 {
            return Err(VariablesError::Unexpected {
                what: "relative handle",
            });
        }
        Ok(h.value)
    }

    fn color(&mut self, version: Version) -> Result<HeaderColor, BitError> {
        let number = self.main.bit_short()?;
        if version < Version::R2004 {
            return Ok(HeaderColor {
                number,
                packed: None,
                name: None,
                book_name: None,
            });
        }
        let packed = self.main.bit_long()? as u32;
        let names = self.main.raw_char()?;
        let name = if names & 1 != 0 {
            Some(self.text()?)
        } else {
            None
        };
        let book_name = if names & 2 != 0 {
            Some(self.text()?)
        } else {
            None
        };
        Ok(HeaderColor {
            number,
            packed: Some(packed),
            name,
            book_name,
        })
    }
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn point3(r: &mut BitReader<'_>) -> Result<[f64; 3], BitError> {
    Ok([r.bit_double()?, r.bit_double()?, r.bit_double()?])
}

fn point2_raw(r: &mut BitReader<'_>) -> Result<[f64; 2], EndOfData> {
    Ok([r.raw_double()?, r.raw_double()?])
}

fn timestamp(r: &mut BitReader<'_>) -> Result<Timestamp, BitError> {
    Ok(Timestamp {
        days: r.bit_long()?,
        milliseconds: r.bit_long()?,
    })
}

/// Reads the header variables from the assembled `AcDb:Header` section (for
/// R2000, the bytes the first section locator names).
pub fn read_header_variables(
    section: &[u8],
    version: Version,
) -> Result<HeaderVariables, VariablesError> {
    let split = match version {
        Version::R2000 | Version::R2004 => false,
        Version::R2007 | Version::R2010 | Version::R2013 | Version::R2018 => true,
    };
    let bad = |what| VariablesError::Unexpected { what };
    if section.len() < 20 || section[..16] != OPENING_SENTINEL {
        return Err(bad("opening sentinel"));
    }
    let size = u32_at(section, 16) as usize;
    // From R2010 an extra count (zero in every file read) may sit between the
    // byte count and the data. The data opens with a bit count that is never
    // zero, so a zero there is the extra count.
    let extra = if split && section.len() >= 24 && u32_at(section, 20) == 0 {
        4
    } else {
        0
    };
    // The section may be followed by bytes the page it was stored in was padded
    // with; the checksum and the closing sentinel mark where the section ends.
    if section.len() < 20 + extra + size + 2 + 16 {
        return Err(bad("length"));
    }
    let region_at = 20 + extra;
    let crc_at = region_at + size;
    let stored = u16::from_le_bytes([section[crc_at], section[crc_at + 1]]);
    let computed = crc16(CRC_SEED, &section[16..crc_at]);
    if computed != stored {
        return Err(VariablesError::Checksum { computed, stored });
    }
    if section[crc_at + 2..crc_at + 18] != CLOSING_SENTINEL {
        return Err(bad("closing sentinel"));
    }
    let region = &section[region_at..crc_at];
    let mut main = BitReader::new(region);

    // Where each stream ends.
    let (main_end, strings, handles, handle_end) = if split {
        let total_bits = main.raw_long()? as usize;
        if total_bits > region.len() * 8 || total_bits < 33 {
            return Err(bad("bit count"));
        }
        // The last bit before the handle stream says whether strings exist;
        // they sit just before the size field(s) that precede that bit.
        let mut probe = BitReader::new(region);
        probe.seek(total_bits - 1);
        if !probe.bit()? {
            return Err(bad("string stream flag"));
        }
        let mut at = total_bits - 1;
        at = at
            .checked_sub(16)
            .ok_or_else(|| bad("string stream size"))?;
        probe.seek(at);
        let mut string_bits = usize::from(probe.raw_short()?);
        if string_bits & 0x8000 != 0 {
            at = at
                .checked_sub(16)
                .ok_or_else(|| bad("string stream size"))?;
            probe.seek(at);
            let high = usize::from(probe.raw_short()?);
            string_bits = (string_bits & 0x7FFF) | (high << 15);
        }
        let start = at
            .checked_sub(string_bits)
            .ok_or_else(|| bad("string stream size"))?;
        let mut s = BitReader::new(region);
        s.seek(start);
        let mut h = BitReader::new(region);
        h.seek(total_bits);
        (start, Some((s, at)), Some(h), region.len() * 8)
    } else {
        (region.len() * 8, None, None, region.len() * 8)
    };
    let (strings, strings_end) = match strings {
        Some((s, end)) => (Some(s), Some(end)),
        None => (None, None),
    };
    let mut st = Streams {
        main,
        strings,
        handles,
    };

    let vars = read_all(&mut st, version)?;

    // Every stream must end where the format puts its end: the main stream at
    // the start of the string stream (or, with no string stream, at the end of
    // the data), the string stream at its size field, and the last stream
    // within one byte of the data's end once padded to a byte boundary -- the
    // writers pad it to a byte, and some leave a whole padding byte more (real
    // files of R2007 and R2010, and one of R2000).
    let main_pos = st.main.position();
    if split {
        if main_pos != main_end {
            return Err(bad("main stream length"));
        }
        if let (Some(s), Some(end)) = (&st.strings, strings_end) {
            if s.position() != end {
                return Err(bad("string stream length"));
            }
        }
        if let Some(h) = &st.handles {
            // Padded to a byte boundary, then (in some files) one whole byte more.
            let aligned = h.position().div_ceil(8) * 8;
            if handle_end.saturating_sub(aligned) > MAX_PADDING_BITS {
                return Err(bad("handle stream length"));
            }
        }
    } else if main_end - main_pos > MAX_PADDING_BITS {
        return Err(bad("data length"));
    }
    Ok(vars)
}

fn read_space(st: &mut Streams<'_>) -> Result<SpaceVariables, VariablesError> {
    let insertion_base = point3(&mut st.main)?;
    let extents_min = point3(&mut st.main)?;
    let extents_max = point3(&mut st.main)?;
    let limits_min = point2_raw(&mut st.main)?;
    let limits_max = point2_raw(&mut st.main)?;
    let elevation = st.main.bit_double()?;
    let ucs_origin = point3(&mut st.main)?;
    let ucs_x_axis = point3(&mut st.main)?;
    let ucs_y_axis = point3(&mut st.main)?;
    let ucs_name = st.handle()?;
    let ucs_ortho_ref = st.handle()?;
    let ucs_ortho_view = st.main.bit_short()?;
    let ucs_base = st.handle()?;
    let mut ucs_ortho_origins = [[0.0; 3]; 6];
    for o in &mut ucs_ortho_origins {
        *o = point3(&mut st.main)?;
    }
    Ok(SpaceVariables {
        insertion_base,
        extents_min,
        extents_max,
        limits_min,
        limits_max,
        elevation,
        ucs_origin,
        ucs_x_axis,
        ucs_y_axis,
        ucs_name,
        ucs_ortho_ref,
        ucs_ortho_view,
        ucs_base,
        ucs_ortho_origins,
    })
}

fn read_dimension(
    st: &mut Streams<'_>,
    version: Version,
) -> Result<DimensionVariables, VariablesError> {
    let r2007 = version >= Version::R2007;
    let r2010 = version >= Version::R2010;
    let post = st.text()?;
    let alt_post = st.text()?;
    let scale = st.main.bit_double()?;
    let arrow_size = st.main.bit_double()?;
    let extension_offset = st.main.bit_double()?;
    let baseline_spacing = st.main.bit_double()?;
    let extension_extend = st.main.bit_double()?;
    let rounding = st.main.bit_double()?;
    let line_extend = st.main.bit_double()?;
    let tolerance_plus = st.main.bit_double()?;
    let tolerance_minus = st.main.bit_double()?;
    let (fixed_extension_length, jog_angle, text_fill, text_fill_color) = if r2007 {
        (
            Some(st.main.bit_double()?),
            Some(st.main.bit_double()?),
            Some(st.main.bit_short()?),
            Some(st.color(version)?),
        )
    } else {
        (None, None, None, None)
    };
    let tolerance = st.main.bit()?;
    let limits = st.main.bit()?;
    let text_inside_horizontal = st.main.bit()?;
    let text_outside_horizontal = st.main.bit()?;
    let suppress_extension_1 = st.main.bit()?;
    let suppress_extension_2 = st.main.bit()?;
    let text_above = st.main.bit_short()?;
    let zero_suppression = st.main.bit_short()?;
    let angular_zero_suppression = st.main.bit_short()?;
    let arc_symbol = if r2007 {
        Some(st.main.bit_short()?)
    } else {
        None
    };
    let text_height = st.main.bit_double()?;
    let center_mark = st.main.bit_double()?;
    let tick_size = st.main.bit_double()?;
    let alt_scale = st.main.bit_double()?;
    let linear_factor = st.main.bit_double()?;
    let text_vertical_position = st.main.bit_double()?;
    let tolerance_height_factor = st.main.bit_double()?;
    let gap = st.main.bit_double()?;
    let alt_rounding = st.main.bit_double()?;
    let alt_units = st.main.bit()?;
    let alt_decimals = st.main.bit_short()?;
    let line_when_outside = st.main.bit()?;
    let separate_arrow_blocks = st.main.bit()?;
    let text_inside = st.main.bit()?;
    let suppress_outside_arrows = st.main.bit()?;
    let line_color = st.color(version)?;
    let extension_color = st.color(version)?;
    let text_color = st.color(version)?;
    let angular_decimals = st.main.bit_short()?;
    let decimals = st.main.bit_short()?;
    let tolerance_decimals = st.main.bit_short()?;
    let alt_unit_format = st.main.bit_short()?;
    let alt_tolerance_decimals = st.main.bit_short()?;
    let angular_unit_format = st.main.bit_short()?;
    let fraction_format = st.main.bit_short()?;
    let linear_unit_format = st.main.bit_short()?;
    let decimal_separator = st.main.bit_short()?;
    let text_move = st.main.bit_short()?;
    let justify = st.main.bit_short()?;
    let suppress_line_1 = st.main.bit()?;
    let suppress_line_2 = st.main.bit()?;
    let tolerance_justify = st.main.bit_short()?;
    let tolerance_zero_suppression = st.main.bit_short()?;
    let alt_zero_suppression = st.main.bit_short()?;
    let alt_tolerance_zero_suppression = st.main.bit_short()?;
    let user_positioned_text = st.main.bit()?;
    let fit = st.main.bit_short()?;
    let fixed_extension_on = if r2007 { Some(st.main.bit()?) } else { None };
    let (
        text_direction,
        alt_multiplier_factor,
        alt_multiplier_suffix,
        multiplier_factor,
        multiplier_suffix,
    ) = if r2010 {
        (
            Some(st.main.bit()?),
            Some(st.main.bit_double()?),
            Some(st.text()?),
            Some(st.main.bit_double()?),
            Some(st.text()?),
        )
    } else {
        (None, None, None, None, None)
    };
    let text_style = st.handle()?;
    let leader_block = st.handle()?;
    let block = st.handle()?;
    let block_1 = st.handle()?;
    let block_2 = st.handle()?;
    let (line_type, extension_line_type_1, extension_line_type_2) = if r2007 {
        (Some(st.handle()?), Some(st.handle()?), Some(st.handle()?))
    } else {
        (None, None, None)
    };
    let line_weight = st.main.bit_short()?;
    let extension_line_weight = st.main.bit_short()?;
    Ok(DimensionVariables {
        post,
        alt_post,
        scale,
        arrow_size,
        extension_offset,
        baseline_spacing,
        extension_extend,
        rounding,
        line_extend,
        tolerance_plus,
        tolerance_minus,
        fixed_extension_length,
        jog_angle,
        text_fill,
        text_fill_color,
        tolerance,
        limits,
        text_inside_horizontal,
        text_outside_horizontal,
        suppress_extension_1,
        suppress_extension_2,
        text_above,
        zero_suppression,
        angular_zero_suppression,
        arc_symbol,
        text_height,
        center_mark,
        tick_size,
        alt_scale,
        linear_factor,
        text_vertical_position,
        tolerance_height_factor,
        gap,
        alt_rounding,
        alt_units,
        alt_decimals,
        line_when_outside,
        separate_arrow_blocks,
        text_inside,
        suppress_outside_arrows,
        line_color,
        extension_color,
        text_color,
        angular_decimals,
        decimals,
        tolerance_decimals,
        alt_unit_format,
        alt_tolerance_decimals,
        angular_unit_format,
        fraction_format,
        linear_unit_format,
        decimal_separator,
        text_move,
        justify,
        suppress_line_1,
        suppress_line_2,
        tolerance_justify,
        tolerance_zero_suppression,
        alt_zero_suppression,
        alt_tolerance_zero_suppression,
        user_positioned_text,
        fit,
        fixed_extension_on,
        text_direction,
        alt_multiplier_factor,
        alt_multiplier_suffix,
        multiplier_factor,
        multiplier_suffix,
        text_style,
        leader_block,
        block,
        block_1,
        block_2,
        line_type,
        extension_line_type_1,
        extension_line_type_2,
        line_weight,
        extension_line_weight,
    })
}

fn read_all(st: &mut Streams<'_>, version: Version) -> Result<HeaderVariables, VariablesError> {
    let r2004 = version >= Version::R2004;
    let r2007 = version >= Version::R2007;
    let r2013 = version >= Version::R2013;
    let required_versions = if r2013 {
        Some(st.main.bit_long_long()?)
    } else {
        None
    };
    // Four doubles and four texts the format does not name, then two longs.
    for _ in 0..4 {
        st.main.bit_double()?;
    }
    for _ in 0..4 {
        st.text()?;
    }
    st.main.bit_long()?;
    st.main.bit_long()?;
    let current_viewport = if version < Version::R2004 {
        Some(st.handle()?)
    } else {
        None
    };
    let dim_associative = st.main.bit()?;
    let dim_show_while_dragging = st.main.bit()?;
    let polyline_generation = st.main.bit()?;
    let ortho_mode = st.main.bit()?;
    let regen_mode = st.main.bit()?;
    let fill_mode = st.main.bit()?;
    let quick_text_mode = st.main.bit()?;
    let paper_space_line_type_scale = st.main.bit()?;
    let limits_check = st.main.bit()?;
    if r2004 {
        st.main.bit()?; // undocumented
    }
    let user_timer = st.main.bit()?;
    let sketch_polylines = st.main.bit()?;
    let angle_direction = st.main.bit()?;
    let spline_frame = st.main.bit()?;
    let mirror_text = st.main.bit()?;
    let world_view = st.main.bit()?;
    let tile_mode = st.main.bit()?;
    let paper_limits_check = st.main.bit()?;
    let retain_xref_visibility = st.main.bit()?;
    let display_silhouettes = st.main.bit()?;
    let polyline_ellipses = st.main.bit()?;
    let proxy_graphics = st.main.bit_short()?;
    let tree_depth = st.main.bit_short()?;
    let linear_units = st.main.bit_short()?;
    let linear_precision = st.main.bit_short()?;
    let angular_units = st.main.bit_short()?;
    let angular_precision = st.main.bit_short()?;
    let attribute_mode = st.main.bit_short()?;
    let point_mode = st.main.bit_short()?;
    if r2004 {
        for _ in 0..3 {
            st.main.bit_long()?;
        }
    }
    let mut user_integers = [0i16; 5];
    for v in &mut user_integers {
        *v = st.main.bit_short()?;
    }
    let spline_segments = st.main.bit_short()?;
    let surface_u = st.main.bit_short()?;
    let surface_v = st.main.bit_short()?;
    let surface_type = st.main.bit_short()?;
    let surface_tab_1 = st.main.bit_short()?;
    let surface_tab_2 = st.main.bit_short()?;
    let spline_type = st.main.bit_short()?;
    let shade_edge = st.main.bit_short()?;
    let shade_diffuse = st.main.bit_short()?;
    let unit_mode = st.main.bit_short()?;
    let max_active_viewports = st.main.bit_short()?;
    let iso_lines = st.main.bit_short()?;
    let mline_justification = st.main.bit_short()?;
    let text_quality = st.main.bit_short()?;
    let line_type_scale = st.main.bit_double()?;
    let text_size = st.main.bit_double()?;
    let trace_width = st.main.bit_double()?;
    let sketch_increment = st.main.bit_double()?;
    let fillet_radius = st.main.bit_double()?;
    let thickness = st.main.bit_double()?;
    let angle_base = st.main.bit_double()?;
    let point_size = st.main.bit_double()?;
    let polyline_width = st.main.bit_double()?;
    let mut user_reals = [0f64; 5];
    for v in &mut user_reals {
        *v = st.main.bit_double()?;
    }
    let mut chamfer = [0f64; 4];
    for v in &mut chamfer {
        *v = st.main.bit_double()?;
    }
    let facet_resolution = st.main.bit_double()?;
    let mline_scale = st.main.bit_double()?;
    let entity_line_type_scale = st.main.bit_double()?;
    let menu_name = st.text()?;
    let created = timestamp(&mut st.main)?;
    let updated = timestamp(&mut st.main)?;
    if r2004 {
        for _ in 0..3 {
            st.main.bit_long()?;
        }
    }
    let time_in_drawing = timestamp(&mut st.main)?;
    let user_timer_value = timestamp(&mut st.main)?;
    let current_color = st.color(version)?;
    // The handle seed is in the main stream, with a length byte first.
    let handle_seed = st.main.handle()?.value;
    let current_layer = st.handle()?;
    let current_text_style = st.handle()?;
    let current_line_type = st.handle()?;
    let current_material = if r2007 { Some(st.handle()?) } else { None };
    let current_dim_style = st.handle()?;
    let current_mline_style = st.handle()?;
    let paper_viewport_scale = st.main.bit_double()?;
    let paper_space = read_space(st)?;
    let model_space = read_space(st)?;
    let dimension = read_dimension(st, version)?;
    let block = st.handle()?;
    let layer = st.handle()?;
    let style = st.handle()?;
    let linetype = st.handle()?;
    let view = st.handle()?;
    let ucs = st.handle()?;
    let vport = st.handle()?;
    let appid = st.handle()?;
    let dimstyle = st.handle()?;
    let viewport_entity_header = if version < Version::R2004 {
        Some(st.handle()?)
    } else {
        None
    };
    let group_dictionary = st.handle()?;
    let mline_style_dictionary = st.handle()?;
    let named_objects = st.handle()?;
    let stack_alignment = st.main.bit_short()?;
    let stack_size = st.main.bit_short()?;
    let hyperlink_base = st.text()?;
    let style_sheet = st.text()?;
    let layouts = st.handle()?;
    let plot_settings = st.handle()?;
    let plot_styles = st.handle()?;
    let (materials, colors) = if r2004 {
        (Some(st.handle()?), Some(st.handle()?))
    } else {
        (None, None)
    };
    let visual_styles = if r2007 { Some(st.handle()?) } else { None };
    let unnamed = if r2013 { Some(st.handle()?) } else { None };
    let flags = st.main.bit_long()?;
    let insertion_units = st.main.bit_short()?;
    let plot_style_type = st.main.bit_short()?;
    let plot_style_id = if plot_style_type == 3 {
        Some(st.handle()?)
    } else {
        None
    };
    let fingerprint_guid = st.text()?;
    let version_guid = st.text()?;
    let (sort_entities, index_control, hide_text) = if r2004 {
        (
            Some(st.main.raw_char()?),
            Some(st.main.raw_char()?),
            Some(st.main.raw_char()?),
        )
    } else {
        (None, None, None)
    };
    let (
        xclip_frame,
        dim_association,
        halo_gap,
        obscured_color,
        intersection_color,
        obscured_line_type,
        intersection_display,
        project_name,
    ) = if r2004 {
        (
            Some(st.main.raw_char()?),
            Some(st.main.raw_char()?),
            Some(st.main.raw_char()?),
            Some(st.main.bit_short()?),
            Some(st.main.bit_short()?),
            Some(st.main.raw_char()?),
            Some(st.main.raw_char()?),
            Some(st.text()?),
        )
    } else {
        (None, None, None, None, None, None, None, None)
    };
    let paper_space_block = st.handle()?;
    let model_space_block = st.handle()?;
    let line_type_by_layer = st.handle()?;
    let line_type_by_block = st.handle()?;
    let line_type_continuous = st.handle()?;
    let extended = if r2007 {
        let camera_display = st.main.bit()?;
        st.main.bit_long()?;
        st.main.bit_long()?;
        st.main.bit_double()?;
        let steps_per_second = st.main.bit_double()?;
        let step_size = st.main.bit_double()?;
        let dwf_precision = st.main.bit_double()?;
        let lens_length = st.main.bit_double()?;
        let camera_height = st.main.bit_double()?;
        let solid_history = st.main.raw_char()?;
        let show_history = st.main.raw_char()?;
        let solid_width = st.main.bit_double()?;
        let solid_height = st.main.bit_double()?;
        let loft_angle_1 = st.main.bit_double()?;
        let loft_angle_2 = st.main.bit_double()?;
        let loft_magnitude_1 = st.main.bit_double()?;
        let loft_magnitude_2 = st.main.bit_double()?;
        let loft_parameters = st.main.bit_short()?;
        let loft_normals = st.main.raw_char()?;
        let latitude = st.main.bit_double()?;
        let longitude = st.main.bit_double()?;
        let north_direction = st.main.bit_double()?;
        let time_zone = st.main.bit_long()?;
        let light_glyph_display = st.main.raw_char()?;
        let tile_model_light_synch = st.main.raw_char()?;
        let dwf_frame = st.main.raw_char()?;
        let dgn_frame = st.main.raw_char()?;
        st.main.bit()?; // unknown
        let interfere_color = st.color(version)?;
        let interfere_object_visual_style = st.handle()?;
        let interfere_viewport_visual_style = st.handle()?;
        let drag_visual_style = st.handle()?;
        let cast_shadows = st.main.raw_char()?;
        st.main.bit_double()?; // unknown
        Some(ExtendedVariables {
            camera_display,
            steps_per_second,
            step_size,
            dwf_precision,
            lens_length,
            camera_height,
            solid_history,
            show_history,
            solid_width,
            solid_height,
            loft_angle_1,
            loft_angle_2,
            loft_magnitude_1,
            loft_magnitude_2,
            loft_parameters,
            loft_normals,
            latitude,
            longitude,
            north_direction,
            time_zone,
            light_glyph_display,
            tile_model_light_synch,
            dwf_frame,
            dgn_frame,
            interfere_color,
            interfere_object_visual_style,
            interfere_viewport_visual_style,
            drag_visual_style,
            cast_shadows,
        })
    } else {
        None
    };
    // Four shorts the format says are not required.
    // They close the last stream: the handle stream where there is one.
    for _ in 0..4 {
        match &mut st.handles {
            Some(h) => h.bit_short()?,
            None => st.main.bit_short()?,
        };
    }
    Ok(HeaderVariables {
        required_versions,
        current_viewport,
        dim_associative,
        dim_show_while_dragging,
        polyline_generation,
        ortho_mode,
        regen_mode,
        fill_mode,
        quick_text_mode,
        paper_space_line_type_scale,
        limits_check,
        user_timer,
        sketch_polylines,
        angle_direction,
        spline_frame,
        mirror_text,
        world_view,
        tile_mode,
        paper_limits_check,
        retain_xref_visibility,
        display_silhouettes,
        polyline_ellipses,
        proxy_graphics,
        tree_depth,
        linear_units,
        linear_precision,
        angular_units,
        angular_precision,
        attribute_mode,
        point_mode,
        user_integers,
        spline_segments,
        surface_u,
        surface_v,
        surface_type,
        surface_tab_1,
        surface_tab_2,
        spline_type,
        shade_edge,
        shade_diffuse,
        unit_mode,
        max_active_viewports,
        iso_lines,
        mline_justification,
        text_quality,
        line_type_scale,
        text_size,
        trace_width,
        sketch_increment,
        fillet_radius,
        thickness,
        angle_base,
        point_size,
        polyline_width,
        user_reals,
        chamfer,
        facet_resolution,
        mline_scale,
        entity_line_type_scale,
        menu_name,
        created,
        updated,
        time_in_drawing,
        user_timer_value,
        current_color,
        handle_seed,
        current_layer,
        current_text_style,
        current_line_type,
        current_material,
        current_dim_style,
        current_mline_style,
        paper_viewport_scale,
        paper_space,
        model_space,
        dimension,
        controls: ControlHandles {
            block,
            layer,
            style,
            linetype,
            view,
            ucs,
            vport,
            appid,
            dimstyle,
            viewport_entity_header,
            group_dictionary,
            mline_style_dictionary,
            named_objects,
            layouts,
            plot_settings,
            plot_styles,
            materials,
            colors,
            visual_styles,
            unnamed,
        },
        stack_alignment,
        stack_size,
        hyperlink_base,
        style_sheet,
        flags,
        insertion_units,
        plot_style_type,
        plot_style_id,
        fingerprint_guid,
        version_guid,
        sort_entities,
        index_control,
        hide_text,
        xclip_frame,
        dim_association,
        halo_gap,
        obscured_color,
        intersection_color,
        obscured_line_type,
        intersection_display,
        project_name,
        paper_space_block,
        model_space_block,
        line_type_by_layer,
        line_type_by_block,
        line_type_continuous,
        extended,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A section of `data`, framed the way the format frames it, with the
    /// checksum the caller gives (or the right one).
    fn framed(data: &[u8], crc: Option<u16>) -> Vec<u8> {
        let mut s = OPENING_SENTINEL.to_vec();
        s.extend_from_slice(&(data.len() as u32).to_le_bytes());
        s.extend_from_slice(data);
        let right = crc16(CRC_SEED, &s[16..]);
        s.extend_from_slice(&crc.unwrap_or(right).to_le_bytes());
        s.extend_from_slice(&CLOSING_SENTINEL);
        s
    }

    #[test]
    fn a_wrong_opening_sentinel_is_refused() {
        let mut s = framed(&[0; 8], None);
        s[0] ^= 1;
        assert_eq!(
            read_header_variables(&s, Version::R2000),
            Err(VariablesError::Unexpected {
                what: "opening sentinel"
            })
        );
    }

    #[test]
    fn a_checksum_that_does_not_match_is_refused() {
        let right = crc16(CRC_SEED, &{
            let mut b = 8u32.to_le_bytes().to_vec();
            b.extend_from_slice(&[0; 8]);
            b
        });
        let s = framed(&[0; 8], Some(right ^ 0x0101));
        assert_eq!(
            read_header_variables(&s, Version::R2000),
            Err(VariablesError::Checksum {
                computed: right,
                stored: right ^ 0x0101
            })
        );
    }

    #[test]
    fn a_stream_that_ends_inside_a_value_is_an_error_not_a_guess() {
        // Right framing and checksum, but eight zero bytes are far too few
        // values for a header.
        assert!(matches!(
            read_header_variables(&framed(&[0; 8], None), Version::R2000),
            Err(VariablesError::Bits(_))
        ));
    }

    #[test]
    fn a_missing_closing_sentinel_is_refused() {
        let mut s = framed(&[0; 8], None);
        let n = s.len();
        s[n - 1] ^= 1;
        assert_eq!(
            read_header_variables(&s, Version::R2000),
            Err(VariablesError::Unexpected {
                what: "closing sentinel"
            })
        );
    }
}
