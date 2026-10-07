//! Sixel graphics DCS sequence handling

//!
//! Capability boundary (ARC-002): these handlers take only the DCS state
//! (and the Sixel limits for the hook), not `&mut Terminal`.

use crate::sixel;
use crate::terminal::DcsState;
use vte::Params;

/// Maximum allowed sixel raster dimension (width or height) in pixels
/// cap: Width or height accepted for a sixel raster declared in a DCS payload.
const MAX_SIXEL_DIMENSION: usize = 16384;

/// Maximum number of sixel color registers
/// cap: Color registers accepted in a sixel palette from a DCS payload.
const MAX_SIXEL_COLORS: usize = 4096;

/// Process accumulated Sixel command from DCS buffer
pub(crate) fn process_sixel_command(dcs_state: &mut DcsState) {
    if dcs_state.dcs_buffer.is_empty() {
        return;
    }

    let Some(parser) = &mut dcs_state.sixel_parser else {
        return;
    };

    let buffer_str = String::from_utf8_lossy(&dcs_state.dcs_buffer);
    let command = buffer_str.chars().next().unwrap_or('\0');

    match command {
            '#' => {
                // Color command: #Pc or #Pc;Pu;Px;Py;Pz
                let params: Vec<&str> = buffer_str[1..].split(';').collect();
                if let Ok(color_idx) = params[0].parse::<usize>() {
                    if color_idx >= MAX_SIXEL_COLORS {
                        // Reject out-of-range color indices
                    } else if params.len() == 1 {
                        // Select color
                        parser.select_color(color_idx);
                    } else if params.len() == 5 {
                        // Define color
                        if let (Ok(color_system), Ok(x), Ok(y), Ok(z)) = (
                            params[1].parse::<u8>(),
                            params[2].parse::<u16>(),
                            params[3].parse::<u16>(),
                            params[4].parse::<u16>(),
                        ) {
                            parser.define_color(color_idx, color_system, x, y, z);
                        }
                    }
                }
            }
            '"' => {
                // Raster attributes: "Pan;Pad;Ph;Pv
                let params: Vec<&str> = buffer_str[1..].split(';').collect();
                if params.len() >= 4 {
                    if let (Ok(pan), Ok(pad), Ok(width), Ok(height)) = (
                        params[0].parse::<u16>(),
                        params[1].parse::<u16>(),
                        params[2].parse::<usize>(),
                        params[3].parse::<usize>(),
                    ) {
                        if width <= MAX_SIXEL_DIMENSION && height <= MAX_SIXEL_DIMENSION {
                            parser.set_raster_attributes(pan, pad, width, height);
                        }
                    }
                }
            }
            '!'
                // Repeat sequence: !Pn character
                if buffer_str.len() >= 2 => {
                    let count_str = &buffer_str[1..buffer_str.len() - 1];
                    let repeat_char = buffer_str.chars().last().unwrap_or('?');
                    if let Ok(count) = count_str.parse::<usize>() {
                        parser.parse_repeat(count, repeat_char);
                    }
                }
            _ => {}
        }

    dcs_state.dcs_buffer.clear();
}

/// DCS hook for Sixel: install a fresh parser configured from the DCS params.
pub(crate) fn handle_sixel_hook(
    dcs_state: &mut DcsState,
    limits: sixel::SixelLimits,
    params: &Params,
) {
    let mut parser = sixel::SixelParser::new_with_limits(limits);

    // Convert Params to Vec<u16> for set_params
    let params_vec: Vec<u16> = params
        .iter()
        .flat_map(|subparams| subparams.iter().copied())
        .collect();

    parser.set_params(&params_vec);

    dcs_state.sixel_parser = Some(parser);
}
