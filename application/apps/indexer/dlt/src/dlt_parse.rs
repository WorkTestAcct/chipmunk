// Copyright (c) 2019 E.S.R.Labs. All rights reserved.
//
// NOTICE:  All information contained herein is, and remains
// the property of E.S.R.Labs and its suppliers, if any.
// The intellectual and technical concepts contained herein are
// proprietary to E.S.R.Labs and its suppliers and may be covered
// by German and Foreign Patents, patents in process, and are protected
// by trade secret or copyright law.
// Dissemination of this information or reproduction of this material
// is strictly forbidden unless prior written permission is obtained
// from E.S.R.Labs.
use crate::dlt::*;
use crate::dlt_net::*;
use crate::filtering;
use crossbeam_channel as cc;
use indexer_base::chunks::{ChunkFactory, ChunkResults};
use indexer_base::config::*;
use indexer_base::error_reporter::*;
use indexer_base::progress::*;
use indexer_base::utils;
use serde::Serialize;

use buf_redux::policy::MinBuffered;
use buf_redux::BufReader as ReduxReader;
use byteorder::{BigEndian, LittleEndian};
use failure::{err_msg, Error};
use nom::bytes::streaming::{tag, take, take_while_m_n};
use nom::{combinator::map, multi::count, number::streaming, sequence::tuple, IResult};
use rustc_hash::FxHashMap;
use std::fs;
use std::io::{BufRead, BufWriter, Read, Write};
use std::rc::Rc;

use crate::fibex::FibexMetadata;
use std::str;

const STOP_CHECK_LINE_THRESHOLD: usize = 250_000;
const DLT_PATTERN_SIZE: usize = 4;
const DLT_PATTERN: &[u8] = &[0x44, 0x4C, 0x54, 0x01];

pub(crate) fn parse_ecu_id(input: &[u8]) -> IResult<&[u8], &str> {
    dlt_zero_terminated_string(input, 4)
}
fn skip_to_next_storage_header<'a, T>(
    input: &'a [u8],
    index: Option<usize>,
    update_channel: Option<&cc::Sender<IndexingResults<T>>>,
) -> Option<&'a [u8]> {
    let mut found = false;
    let mut to_drop = 0usize;
    for v in input.windows(4) {
        if v == DLT_PATTERN {
            found = true;
            break;
        }
        to_drop += 1;
    }

    if !found {
        debug!(
            "did not find another storage header (input left {})",
            input.len()
        );
        if let Some(tx) = update_channel {
            let _ = tx.send(Err(Notification {
                severity: Severity::ERROR,
                content: "did not find another storage header".to_string(),
                line: index,
            }));
        }
        return None;
    }
    if to_drop > 0 {
        if let Some(tx) = update_channel {
            let _ = tx.send(Err(Notification {
                severity: Severity::ERROR,
                content: format!("dropped {} to get to next message", to_drop),
                line: index,
            }));
        }
        // println!("dropped {} to get to next message", to_drop);
    }
    Some(&input[to_drop..])
}
fn dlt_skip_storage_header<'a, T>(
    input: &'a [u8],
    index: Option<usize>,
    update_channel: Option<&cc::Sender<IndexingResults<T>>>,
) -> IResult<&'a [u8], ()> {
    // println!("dlt_skip_storage_header");
    match skip_to_next_storage_header(input, index, update_channel) {
        Some(rest) => {
            let (i, (_, _, _)): (&'a [u8], _) =
                tuple((tag("DLT"), tag(&[0x01]), take(12usize)))(rest)?;
            Ok((i, ()))
        }
        None => Err(nom::Err::Error((&[], nom::error::ErrorKind::Verify))),
    }
}
pub(crate) fn dlt_storage_header<'a, T>(
    input: &'a [u8],
    index: Option<usize>,
    update_channel: Option<&cc::Sender<IndexingResults<T>>>,
) -> IResult<&'a [u8], Option<StorageHeader>> {
    // println!("dlt_storage_header (left: {} bytes)", input.len());
    match skip_to_next_storage_header(input, index, update_channel) {
        Some(rest) => {
            // println!("rest is {} bytes", rest.len());
            let (i, (_, _, seconds, microseconds)) = tuple((
                tag("DLT"),
                tag(&[0x01]),
                streaming::le_u32,
                streaming::le_u32,
            ))(rest)?;
            let (after_string, ecu_id) = dlt_zero_terminated_string(i, 4)?;
            // println!("after_stringA (left: {} bytes)", after_string.len());
            Ok((
                after_string,
                Some(StorageHeader {
                    timestamp: DltTimeStamp {
                        seconds,
                        microseconds,
                    },
                    ecu_id: ecu_id.to_string(),
                }),
            ))
        }
        None => Err(nom::Err::Failure((&[], nom::error::ErrorKind::Verify))),
    }
}

fn maybe_parse_ecu_id(a: bool) -> impl Fn(&[u8]) -> IResult<&[u8], Option<&str>> {
    fn parse_ecu_id_to_option(input: &[u8]) -> IResult<&[u8], Option<&str>> {
        map(parse_ecu_id, Some)(input)
    }
    fn parse_nothing_str(input: &[u8]) -> IResult<&[u8], Option<&str>> {
        Ok((input, None))
    }
    if a {
        parse_ecu_id_to_option
    } else {
        parse_nothing_str
    }
}
fn maybe_parse_u32(a: bool) -> impl Fn(&[u8]) -> IResult<&[u8], Option<u32>> {
    fn parse_u32_to_option(input: &[u8]) -> IResult<&[u8], Option<u32>> {
        map(streaming::be_u32, Some)(input)
    }
    fn parse_nothing_u32(input: &[u8]) -> IResult<&[u8], Option<u32>> {
        Ok((input, None))
    }
    if a {
        parse_u32_to_option
    } else {
        parse_nothing_u32
    }
}

/// The standard header is part of every DLT message
/// all big endian format [PRS_Dlt_00091]
pub(crate) fn dlt_standard_header(input: &[u8]) -> IResult<&[u8], StandardHeader> {
    let (rest, header_type_byte) = streaming::be_u8(input)?;
    let has_ecu_id = (header_type_byte & WITH_ECU_ID_FLAG) != 0;
    let has_session_id = (header_type_byte & WITH_SESSION_ID_FLAG) != 0;
    let has_timestamp = (header_type_byte & WITH_TIMESTAMP_FLAG) != 0;
    let (i, (message_counter, overall_length, ecu_id, session_id, timestamp)) = tuple((
        streaming::be_u8,
        streaming::be_u16,
        maybe_parse_ecu_id(has_ecu_id),
        maybe_parse_u32(has_session_id),
        maybe_parse_u32(has_timestamp),
    ))(rest)?;
    let has_extended_header = (header_type_byte & WITH_EXTENDED_HEADER_FLAG) != 0;
    let payload_length = overall_length - calculate_all_headers_length(header_type_byte);

    Ok((
        i,
        StandardHeader::new(
            header_type_byte >> 5 & 0b111,
            if (header_type_byte & BIG_ENDIAN_FLAG) != 0 {
                Endianness::Big
            } else {
                Endianness::Little
            },
            message_counter,
            has_extended_header,
            payload_length,
            ecu_id.map(|r| r.to_string()),
            session_id,
            timestamp,
        ),
    ))
}

pub(crate) fn dlt_extended_header<'a, T>(
    input: &'a [u8],
    index: Option<usize>,
    update_channel: Option<&cc::Sender<IndexingResults<T>>>,
) -> IResult<&'a [u8], ExtendedHeader> {
    let (i, (message_info, argument_count, app_id, context_id)) = tuple((
        streaming::be_u8,
        streaming::be_u8,
        parse_ecu_id,
        parse_ecu_id,
    ))(input)?;
    let verbose = (message_info & VERBOSE_FLAG) != 0;
    match MessageType::try_from(message_info) {
        Ok(message_type) => {
            if let Some(tx) = update_channel {
                match message_type {
                    MessageType::Unknown(n) => {
                        let _ = tx.send(Err(Notification {
                            severity: Severity::WARNING,
                            content: format!("unknown message type {:?}", n),
                            line: index,
                        }));
                    }
                    MessageType::Log(LogLevel::Invalid(n)) => {
                        let _ = tx.send(Err(Notification {
                            severity: Severity::WARNING,
                            content: format!("unknown log level {}", n),
                            line: index,
                        }));
                    }
                    MessageType::Control(ControlType::Unknown(n)) => {
                        let _ = tx.send(Err(Notification {
                            severity: Severity::WARNING,
                            content: format!("unknown control type {}", n),
                            line: index,
                        }));
                    }
                    MessageType::ApplicationTrace(ApplicationTraceType::Invalid(n)) => {
                        let _ = tx.send(Err(Notification {
                            severity: Severity::WARNING,
                            content: format!("invalid application-trace type {}", n),
                            line: index,
                        }));
                    }
                    MessageType::NetworkTrace(NetworkTraceType::Invalid) => {
                        let _ = tx.send(Err(Notification {
                            severity: Severity::WARNING,
                            content: "invalid application-trace type 0".to_string(),
                            line: index,
                        }));
                    }
                    _ => (),
                };
            };
            Ok((
                i,
                ExtendedHeader {
                    verbose,
                    argument_count,
                    message_type,
                    application_id: app_id.to_string(),
                    context_id: context_id.to_string(),
                },
            ))
        }
        Err(e) => {
            if let Some(tx) = update_channel {
                let _ = tx.send(Err(Notification {
                    severity: Severity::ERROR,
                    content: format!("lineInvalid message type: {}", e),
                    line: index,
                }));
            }

            let err_ctx: (&[u8], nom::error::ErrorKind) = (&[], nom::error::ErrorKind::Verify);
            let err = nom::Err::Error(err_ctx);
            Err(err)
        }
    }
}
#[inline]
pub fn is_not_null(chr: u8) -> bool {
    chr != 0x0
}
pub fn dlt_zero_terminated_string(s: &[u8], size: usize) -> IResult<&[u8], &str> {
    let (rest_with_null, content_without_null) = take_while_m_n(0, size, is_not_null)(s)?;
    let res_str = match nom::lib::std::str::from_utf8(content_without_null) {
        Ok(content) => content,
        Err(e) => {
            let (valid, _) = content_without_null.split_at(e.valid_up_to());
            unsafe { nom::lib::std::str::from_utf8_unchecked(valid) }
        }
    };
    let missing = size - content_without_null.len();
    let (rest, _) = take(missing)(rest_with_null)?;
    Ok((rest, res_str))
}

#[allow(clippy::type_complexity)]
fn dlt_variable_name_and_unit<T: NomByteOrder>(
    type_info: &TypeInfo,
) -> fn(&[u8]) -> IResult<&[u8], (Option<String>, Option<String>)> {
    // println!("dlt_variable_name_and_unit");
    if type_info.has_variable_info {
        |input| {
            let (i2, (name_size, unit_size)) = tuple((T::parse_u16, T::parse_u16))(input)?;
            dbg_parsed("namesize, unitsize", input, i2);
            // println!("(name_size, unit_size): {:?}", (name_size, unit_size));
            let (i3, name) = dlt_zero_terminated_string(i2, name_size as usize)?;
            dbg_parsed("name", i2, i3);
            // println!("name: {}", name);
            let (rest, unit) = dlt_zero_terminated_string(i3, unit_size as usize)?;
            dbg_parsed("unit", i3, rest);
            // println!("unit: {}", unit);
            Ok((rest, (Some(name.to_string()), Some(unit.to_string()))))
        }
    } else {
        |input| Ok((input, (None, None)))
    }
}
fn dlt_variable_name<T: NomByteOrder>(input: &[u8]) -> IResult<&[u8], String> {
    let (i, size) = T::parse_u16(input)?;
    let (i2, name) = dlt_zero_terminated_string(i, size as usize)?;
    Ok((i2, name.to_string()))
}
pub trait NomByteOrder: Clone + Copy + Eq + Ord + PartialEq + PartialOrd {
    fn parse_u16(i: &[u8]) -> IResult<&[u8], u16>;
    fn parse_i16(i: &[u8]) -> IResult<&[u8], i16>;
    fn parse_u32(i: &[u8]) -> IResult<&[u8], u32>;
    fn parse_i32(i: &[u8]) -> IResult<&[u8], i32>;
    fn parse_f32(i: &[u8]) -> IResult<&[u8], f32>;
    fn parse_u64(i: &[u8]) -> IResult<&[u8], u64>;
    fn parse_i64(i: &[u8]) -> IResult<&[u8], i64>;
    fn parse_f64(i: &[u8]) -> IResult<&[u8], f64>;
    fn parse_u128(i: &[u8]) -> IResult<&[u8], u128>;
    fn parse_i128(i: &[u8]) -> IResult<&[u8], i128>;
}

impl NomByteOrder for BigEndian {
    #[inline]
    fn parse_u16(i: &[u8]) -> IResult<&[u8], u16> {
        streaming::be_u16(i)
    }
    #[inline]
    fn parse_i16(i: &[u8]) -> IResult<&[u8], i16> {
        streaming::be_i16(i)
    }
    #[inline]
    fn parse_u32(i: &[u8]) -> IResult<&[u8], u32> {
        streaming::be_u32(i)
    }
    #[inline]
    fn parse_i32(i: &[u8]) -> IResult<&[u8], i32> {
        streaming::be_i32(i)
    }
    #[inline]
    fn parse_f32(i: &[u8]) -> IResult<&[u8], f32> {
        streaming::be_f32(i)
    }
    #[inline]
    fn parse_u64(i: &[u8]) -> IResult<&[u8], u64> {
        streaming::be_u64(i)
    }
    #[inline]
    fn parse_i64(i: &[u8]) -> IResult<&[u8], i64> {
        streaming::be_i64(i)
    }
    #[inline]
    fn parse_f64(i: &[u8]) -> IResult<&[u8], f64> {
        streaming::be_f64(i)
    }
    #[inline]
    fn parse_u128(i: &[u8]) -> IResult<&[u8], u128> {
        streaming::be_u128(i)
    }
    #[inline]
    fn parse_i128(i: &[u8]) -> IResult<&[u8], i128> {
        streaming::be_i128(i)
    }
}

impl NomByteOrder for LittleEndian {
    #[inline]
    fn parse_u16(i: &[u8]) -> IResult<&[u8], u16> {
        streaming::le_u16(i)
    }
    #[inline]
    fn parse_i16(i: &[u8]) -> IResult<&[u8], i16> {
        streaming::le_i16(i)
    }
    #[inline]
    fn parse_u32(i: &[u8]) -> IResult<&[u8], u32> {
        streaming::le_u32(i)
    }
    #[inline]
    fn parse_i32(i: &[u8]) -> IResult<&[u8], i32> {
        streaming::le_i32(i)
    }
    #[inline]
    fn parse_f32(i: &[u8]) -> IResult<&[u8], f32> {
        streaming::le_f32(i)
    }
    #[inline]
    fn parse_u64(i: &[u8]) -> IResult<&[u8], u64> {
        streaming::le_u64(i)
    }
    #[inline]
    fn parse_i64(i: &[u8]) -> IResult<&[u8], i64> {
        streaming::le_i64(i)
    }
    #[inline]
    fn parse_f64(i: &[u8]) -> IResult<&[u8], f64> {
        streaming::le_f64(i)
    }
    #[inline]
    fn parse_u128(i: &[u8]) -> IResult<&[u8], u128> {
        streaming::le_u128(i)
    }
    #[inline]
    fn parse_i128(i: &[u8]) -> IResult<&[u8], i128> {
        streaming::le_i128(i)
    }
}

pub(crate) fn dlt_uint<T: NomByteOrder>(width: TypeLength) -> fn(&[u8]) -> IResult<&[u8], Value> {
    // println!("dlt_uint ...");
    match width {
        TypeLength::BitLength8 => |i| map(streaming::be_u8, Value::U8)(i),
        TypeLength::BitLength16 => |i| map(T::parse_u16, Value::U16)(i),
        TypeLength::BitLength32 => |i| map(T::parse_u32, Value::U32)(i),
        TypeLength::BitLength64 => |i| map(T::parse_u64, Value::U64)(i),
        TypeLength::BitLength128 => |i| map(T::parse_u128, Value::U128)(i),
    }
}
pub(crate) fn dlt_sint<T: NomByteOrder>(width: TypeLength) -> fn(&[u8]) -> IResult<&[u8], Value> {
    match width {
        TypeLength::BitLength8 => |i| map(streaming::be_i8, Value::I8)(i),
        TypeLength::BitLength16 => |i| map(T::parse_i16, Value::I16)(i),
        TypeLength::BitLength32 => |i| map(T::parse_i32, Value::I32)(i),
        TypeLength::BitLength64 => |i| map(T::parse_i64, Value::I64)(i),
        TypeLength::BitLength128 => |i| map(T::parse_i128, Value::I128)(i),
    }
}
pub(crate) fn dlt_fint<T: NomByteOrder>(width: FloatWidth) -> fn(&[u8]) -> IResult<&[u8], Value> {
    match width {
        FloatWidth::Width32 => |i| map(T::parse_f32, Value::F32)(i),
        FloatWidth::Width64 => |i| map(T::parse_f64, Value::F64)(i),
    }
}
pub(crate) fn dlt_type_info<T: NomByteOrder>(input: &[u8]) -> IResult<&[u8], TypeInfo> {
    let (i, info) = T::parse_u32(input)?;
    match TypeInfo::try_from(info) {
        Ok(type_info) => Ok((i, type_info)),
        Err(_) => {
            report_error(format!("dlt_type_info no type_info for 0x{:02X?}", info));
            Err(nom::Err::Error((&[], nom::error::ErrorKind::Verify)))
        }
    }
}
pub(crate) fn dlt_fixed_point<T: NomByteOrder>(
    input: &[u8],
    width: FloatWidth,
) -> IResult<&[u8], FixedPoint> {
    // println!("width {:?} dlt_fixedpoint,input: \t{:02X?}", width, input);
    let (i, quantization) = T::parse_f32(input)?;
    // println!("parsed quantization: {:?}", quantization);
    if width == FloatWidth::Width32 {
        let (rest, offset) = T::parse_i32(i)?;
        // println!("parsed offset: {:?}", offset);
        Ok((
            rest,
            FixedPoint {
                quantization,
                offset: FixedPointValue::I32(offset),
            },
        ))
    } else if width == FloatWidth::Width64 {
        let (rest, offset) = T::parse_i64(i)?;
        Ok((
            rest,
            FixedPoint {
                quantization,
                offset: FixedPointValue::I64(offset),
            },
        ))
    } else {
        report_error("error in dlt_fixed_point");
        Err(nom::Err::Error((&[], nom::error::ErrorKind::Verify)))
    }
}
pub(crate) fn dlt_argument<T: NomByteOrder>(input: &[u8]) -> IResult<&[u8], Argument> {
    let (i, type_info) = dlt_type_info::<T>(input)?;
    dbg_parsed("type info", input, i);
    // println!("type info: {:?}", type_info);
    match type_info.kind {
        TypeInfoKind::Signed(width) => {
            let (before_val, (name, unit)) = dlt_variable_name_and_unit::<T>(&type_info)(i)?;
            dbg_parsed("name and unit", i, before_val);
            let (after_fixed_point, fixed_point) = (before_val, None);
            dbg_parsed("fixed_point", before_val, after_fixed_point);
            let (rest, value) = dlt_sint::<T>(width)(after_fixed_point)?;
            Ok((
                rest,
                Argument {
                    name,
                    unit,
                    value,
                    fixed_point,
                    type_info,
                },
            ))
        }
        TypeInfoKind::SignedFixedPoint(width) => {
            // println!("parsing TypeInfoKind::Signed");
            let (before_val, (name, unit)) = dlt_variable_name_and_unit::<T>(&type_info)(i)?;
            dbg_parsed("name and unit", i, before_val);
            let (r, fp) = dlt_fixed_point::<T>(before_val, width)?;
            let (after_fixed_point, fixed_point) = (r, Some(fp));
            dbg_parsed("fixed_point", before_val, after_fixed_point);
            let (rest, value) =
                dlt_sint::<T>(float_width_to_type_length(width))(after_fixed_point)?;
            Ok((
                rest,
                Argument {
                    name,
                    unit,
                    value,
                    fixed_point,
                    type_info,
                },
            ))
        }
        TypeInfoKind::Unsigned(width) => {
            let (before_val, (name, unit)) = dlt_variable_name_and_unit::<T>(&type_info)(i)?;
            // println!("Unsigned: calling dlt_uint for {:02X?}", before_val);
            let (rest, value) = dlt_uint::<T>(width)(before_val)?;
            Ok((
                rest,
                Argument {
                    name,
                    unit,
                    value,
                    fixed_point: None,
                    type_info,
                },
            ))
        }
        TypeInfoKind::UnsignedFixedPoint(width) => {
            let (before_val, (name, unit)) = dlt_variable_name_and_unit::<T>(&type_info)(i)?;
            let (after_fixed_point, fixed_point) = {
                let (r, fp) = dlt_fixed_point::<T>(before_val, width)?;
                (r, Some(fp))
            };
            // println!(
            //     "UnsignedFixedPoint: calling dlt_uint for {:02X?}",
            //     before_val
            // );
            let (rest, value) =
                dlt_uint::<T>(float_width_to_type_length(width))(after_fixed_point)?;
            Ok((
                rest,
                Argument {
                    name,
                    unit,
                    value,
                    fixed_point,
                    type_info,
                },
            ))
        }
        TypeInfoKind::Float(width) => {
            let (rest, ((name, unit), value)) = tuple((
                dlt_variable_name_and_unit::<T>(&type_info),
                dlt_fint::<T>(width),
            ))(i)?;
            Ok((
                rest,
                Argument {
                    name,
                    unit,
                    value,
                    fixed_point: None,
                    type_info,
                },
            ))
        }
        TypeInfoKind::Raw => {
            let (i2, raw_byte_cnt) = T::parse_u16(i)?;
            let (i3, name) = if type_info.has_variable_info {
                map(dlt_variable_name::<T>, Some)(i2)?
            } else {
                (i2, None)
            };
            let (rest, value) = map(take(raw_byte_cnt), |s: &[u8]| Value::Raw(s.to_vec()))(i3)?;
            Ok((
                rest,
                Argument {
                    name,
                    unit: None,
                    value,
                    fixed_point: None,
                    type_info,
                },
            ))
        }
        TypeInfoKind::Bool => {
            let (after_var_name, name) = if type_info.has_variable_info {
                map(dlt_variable_name::<T>, Some)(i)?
            } else {
                (i, None)
            };
            dbg_parsed("var name", i, after_var_name);
            let (rest, bool_value) = streaming::be_u8(after_var_name)?;
            dbg_parsed("bool value", after_var_name, rest);
            Ok((
                rest,
                Argument {
                    type_info,
                    name,
                    unit: None,
                    fixed_point: None,
                    value: Value::Bool(bool_value != 0),
                },
            ))
        }
        TypeInfoKind::StringType => {
            let (i2, size) = T::parse_u16(i)?;
            let (i3, name) = if type_info.has_variable_info {
                map(dlt_variable_name::<T>, Some)(i2)?
            } else {
                (i2, None)
            };
            let (rest, value) = dlt_zero_terminated_string(i3, size as usize)?;
            // println!(
            //     "was stringtype: \"{}\", size should have been {}",
            //     value, size
            // );
            Ok((
                rest,
                Argument {
                    name,
                    unit: None,
                    fixed_point: None,
                    value: Value::StringVal(value.to_string()),
                    type_info,
                },
            ))
        }
    }
}

#[allow(dead_code)]
struct DltArgumentParser {
    current_index: Option<usize>,
}

fn dlt_payload<T: NomByteOrder>(
    input: &[u8],
    verbose: bool,
    payload_length: u16,
    arg_cnt: u8,
    is_controll_msg: bool,
) -> IResult<&[u8], Payload2> {
    // println!("try to parse dlt_payload for {:02X?}", input,);
    if verbose {
        // println!("verbose, arg_cnt = {}", arg_cnt);
        let (rest, arguments) = count(dlt_argument::<T>, arg_cnt as usize)(input)?;
        Ok((
            rest,
            Payload2 {
                payload_content: PayloadContent::Verbose(arguments),
            },
        ))
    } else if is_controll_msg {
        // println!("is_controll_msg");
        if payload_length < 1 {
            // println!("error, payload too short {}", payload_length);
            return Err(nom::Err::Failure((&[], nom::error::ErrorKind::Verify)));
        }
        match tuple((nom::number::complete::be_u8, take(payload_length - 1)))(input) {
            Ok((rest, (control_msg_id, payload))) => Ok((
                rest,
                Payload2 {
                    payload_content: PayloadContent::ControlMsg(
                        ControlType::from_value(control_msg_id),
                        payload.to_vec(),
                    ),
                },
            )),
            Err(e) => {
                // println!("error e {:?}", e);
                Err(e)
            }
        }
    } else {
        // println!("non verbose (input.len = {})", input.len());
        // println!(
        //     "not is_controll_msg, payload_length: {}, input left: {}",
        //     payload_length,
        //     input.len()
        // );
        if input.len() < 4 {
            // println!("error, payload too short {}", input.len());
            return Err(nom::Err::Failure((&[], nom::error::ErrorKind::Verify)));
        }
        match tuple((T::parse_u32, take(payload_length - 4)))(input) {
            Ok((rest, (message_id, payload))) => Ok((
                rest,
                Payload2 {
                    payload_content: PayloadContent::NonVerbose(message_id, payload.to_vec()),
                },
            )),
            Err(e) => {
                // println!("error e {:?}", e);
                Err(e)
            }
        }
    }
}

#[inline]
fn dbg_parsed(_name: &str, _before: &[u8], _after: &[u8]) {
    // #[cfg(feature = "debug_parser")]
    {
        let input_len = _before.len();
        let now_len = _after.len();
        let parsed_len = input_len - now_len;
        if parsed_len == 0 {
            trace!("{}: not parsed", _name);
        } else {
            trace!(
                "parsed {} ({} bytes): {:02X?}",
                _name,
                parsed_len,
                &_before[0..parsed_len]
            );
        }
    }
}
/// a DLT message looks like this: [STANDARD-HEADER][EXTENDED-HEADER][PAYLOAD]
/// if stored, an additional header is placed BEFORE all of this [storage-header][...]
/// example: 444C5401 262CC94D D8A20C00 45435500 3500001F 45435500 3F88623A 16014150 5000434F 4E001100 00000472 656D6F
/// --------------------------------------------
/// [STORAGE-HEADER]: 444C5401 262CC94D D8A20C00 45435500
///     444C5401 = DLT + 0x01 (DLT Pattern)
///  timestamp_sec: 262CC94D = 0x4DC92C26
///  timestamp_us: D8A20C00 = 0x000CA2D8
///  ecu-id: 45435500 = b"ECU\0"
///
/// 3500001F 45435500 3F88623A 16014150 5000434F 4E001100 00000472 656D6F (31 byte)
/// --------------------------------------------
/// [HEADER]: 35 00 001F 45435500 3F88623A
///   header type = 0x35 = 0b0011 0101
///       UEH: 1 - > using extended header
///       MSBF: 0 - > little endian
///       WEID: 1 - > with ecu id
///       WSID: 0 - > no session id
///       WTMS: 1 - > with timestamp
///   message counter = 0x00 = 0
///   length = 001F = 31
///   ecu-id = 45435500 = "ECU "
///   timestamp = 3F88623A = 106590265.0 ms since ECU startup (~30 h)
/// --------------------------------------------
/// [EXTENDED HEADER]: 16014150 5000434F 4E00
///   message-info MSIN = 0x16 = 0b0001 0110
///   0 -> non-verbose
///   011 (MSTP Message Type) = 0x3 = Dlt Control Message
///   0001 (MTIN Message Type Info) = 0x1 = Request Control Message
///   number of arguments NOAR = 0x01
///   application id = 41505000 = "APP "
///   context id = 434F4E00 = "CON "
/// --------------------------------------------
/// payload: 1100 00000472 656D6F
///   0x11 == SetDefaultLogLevel
///     00 == new log level (block all messages)
///
pub fn dlt_message<'a>(
    input: &'a [u8],
    filter_config_opt: Option<&filtering::ProcessedDltFilterConfig>,
    index: usize,
    update_channel: Option<&cc::Sender<ChunkResults>>,
    fibex_metadata: Option<Rc<FibexMetadata>>,
    with_storage_header: bool,
) -> IResult<&'a [u8], Option<Message>> {
    // trace!("starting to parse dlt_message==================");
    let (after_storage_header, storage_header) = if with_storage_header {
        dlt_storage_header(input, Some(index), update_channel)?
    } else {
        (input, None)
    };
    dbg_parsed("storage header", &input, &after_storage_header);
    // trace!("dlt_msg 2");
    let (after_storage_and_normal_header, header) = dlt_standard_header(after_storage_header)?;
    // trace!(
    //     "parsed header is {}",
    //     if header.endianness == Endianness::Big {
    //         "big endian"
    //     } else {
    //         "little endian"
    //     }
    // );
    dbg_parsed(
        "normal header",
        &after_storage_header,
        &after_storage_and_normal_header,
    );
    // trace!("dlt_msg 3, header: {:?}", serde_json::to_string(&header));

    // trace!("parsing 2...let's validate the payload length");
    let payload_length = match validated_payload_length(&header, Some(index), update_channel) {
        Some(length) => length,
        None => {
            return Ok((after_storage_and_normal_header, None));
        }
    };

    // trace!("dlt_msg 4, payload_length: {}", payload_length);
    let mut verbose: bool = false;
    let mut is_controll_msg = false;
    let mut arg_count = 0;
    let (after_headers, extended_header) = if header.has_extended_header {
        // trace!("try to parse extended header");
        let (rest, ext_header) =
            dlt_extended_header(after_storage_and_normal_header, Some(index), update_channel)?;
        verbose = ext_header.verbose;
        arg_count = ext_header.argument_count;
        // trace!(
        //     "did parse extended header (type: {})",
        //     ext_header.message_type
        // );
        is_controll_msg = match ext_header.message_type {
            MessageType::Control(_) => true,
            _ => false,
        };
        // trace!(
        //     "did parse extended header, verbose: {}, arg_count: {}, is_controll: {}",
        //     verbose,
        //     arg_count,
        //     is_controll_msg
        // );
        (rest, Some(ext_header))
    } else {
        (after_storage_and_normal_header, None)
    };
    dbg_parsed(
        "extended header",
        &after_storage_and_normal_header,
        &after_headers,
    );
    // trace!(
    //     "extended header: {:?}",
    //     serde_json::to_string(&extended_header)
    // );
    // trace!("dlt_msg 5");
    if let Some(filter_config) = filter_config_opt {
        // trace!("dlt_msg 6");
        if let Some(h) = &extended_header {
            // trace!("dlt_msg 7");
            if let Some(min_filter_level) = filter_config.min_log_level {
                if h.skip_with_level(min_filter_level) {
                    // trace!("no need to parse further, skip payload (skipped level)");
                    let (after_message, _) = take(payload_length)(after_headers)?;
                    return Ok((after_message, None));
                }
            }
            if let Some(only_these_components) = &filter_config.app_ids {
                if !only_these_components.contains(&h.application_id) {
                    // trace!("no need to parse further, skip payload (skipped app id)");
                    let (after_message, _) = take(payload_length)(after_headers)?;
                    return Ok((after_message, None));
                }
            }
            if let Some(only_these_context_ids) = &filter_config.context_ids {
                if !only_these_context_ids.contains(&h.context_id) {
                    // trace!("no need to parse further, skip payload (skipped context id)");
                    let (after_message, _) = take(payload_length)(after_headers)?;
                    return Ok((after_message, None));
                }
            }
            if let Some(only_these_ecu_ids) = &filter_config.ecu_ids {
                if let Some(ecu_id) = &header.ecu_id {
                    if !only_these_ecu_ids.contains(ecu_id) {
                        // trace!("no need to parse further, skip payload (skipped ecu id)");
                        let (after_message, _) = take(payload_length)(after_headers)?;
                        return Ok((after_message, None));
                    }
                }
            }
        }
    }
    // trace!("about to parse payload, left: {}", after_headers.len());
    // trace!("after_headers: {} bytes left", after_headers.len());
    // trace!(
    //     "parsing payload (header is {})",
    //     if header.endianness == Endianness::Big {
    //         "big endian"
    //     } else {
    //         "little endian"
    //     }
    // );
    let (i, payload) = if header.endianness == Endianness::Big {
        // trace!("parsing payload big endian");
        dlt_payload::<BigEndian>(
            after_headers,
            verbose,
            payload_length,
            arg_count,
            is_controll_msg,
        )?
    } else {
        // trace!("parsing payload little endian");
        dlt_payload::<LittleEndian>(
            after_headers,
            verbose,
            payload_length,
            arg_count,
            is_controll_msg,
        )?
    };
    dbg_parsed("payload", &after_headers, &i);
    // trace!("after payload: {} bytes left", i.len());
    Ok((
        i,
        Some(Message {
            storage_header,
            header,
            extended_header,
            payload,
            fibex_metadata,
        }),
    ))
}
fn validated_payload_length<T>(
    header: &StandardHeader,
    index: Option<usize>,
    update_channel: Option<&cc::Sender<IndexingResults<T>>>,
) -> Option<u16> {
    let message_length = header.overall_length();
    let headers_length = calculate_all_headers_length(header.header_type_byte());
    if message_length < headers_length {
        if let Some(tx) = update_channel {
            let _ = tx.send(Err(Notification {
                severity: Severity::ERROR,
                content: format!(
                    "Invalid header length {} (message only has {} bytes)",
                    headers_length, message_length
                ),
                line: index,
            }));
        }
        return None;
    }
    Some(message_length - headers_length)
}
pub fn dlt_statistic_row_info<'a, T>(
    input: &'a [u8],
    index: Option<usize>,
    update_channel: Option<&cc::Sender<IndexingResults<T>>>,
) -> IResult<&'a [u8], StatisticRowInfo> {
    let update_channel_ref = update_channel;
    let (after_storage_header, _) = dlt_skip_storage_header(input, index, update_channel_ref)?;
    let (after_storage_and_normal_header, header) = dlt_standard_header(after_storage_header)?;

    let payload_length = match validated_payload_length(&header, index, update_channel_ref) {
        Some(length) => length,
        None => {
            return Ok((
                after_storage_and_normal_header,
                StatisticRowInfo {
                    app_id_context_id: None,
                    ecu_id: header.ecu_id,
                    level: None,
                    verbose: false,
                },
            ));
        }
    };
    if !header.has_extended_header {
        // no app id, skip rest
        let (after_message, _) = take(payload_length)(after_storage_and_normal_header)?;
        return Ok((
            after_message,
            StatisticRowInfo {
                app_id_context_id: None,
                ecu_id: header.ecu_id,
                level: None,
                verbose: false,
            },
        ));
    }

    let (after_headers, extended_header) =
        dlt_extended_header(after_storage_and_normal_header, index, update_channel)?;
    // skip payload
    let (after_message, _) = take(payload_length)(after_headers)?;
    let level = match extended_header.message_type {
        MessageType::Log(level) => Some(level),
        _ => None,
    };
    Ok((
        after_message,
        StatisticRowInfo {
            app_id_context_id: Some((extended_header.application_id, extended_header.context_id)),
            ecu_id: header.ecu_id,
            level,
            verbose: extended_header.verbose,
        },
    ))
}

#[derive(Debug, Fail)]
pub enum DltParseError {
    #[fail(display = "parsing stopped, cannot continue: {}", cause)]
    Unrecoverable { cause: String },
    #[fail(display = "parsing error, try to continue: {}", reason)]
    ParsingHickup { reason: String },
}
impl From<std::io::Error> for DltParseError {
    fn from(err: std::io::Error) -> DltParseError {
        DltParseError::Unrecoverable {
            cause: format!("{}", err),
        }
    }
}

pub struct FileMessageProducer {
    reader: ReduxReader<fs::File, MinBuffered>,
    filter_config: Option<filtering::ProcessedDltFilterConfig>,
    index: usize,
    update_channel: cc::Sender<ChunkResults>,
    with_storage_header: bool,
}

impl FileMessageProducer {
    fn new(
        in_file: &std::path::PathBuf,
        filter_config: Option<filtering::ProcessedDltFilterConfig>,
        index: usize,
        update_channel: cc::Sender<ChunkResults>,
        with_storage_header: bool,
    ) -> Result<FileMessageProducer, Error> {
        let f = match fs::File::open(&in_file) {
            Ok(file) => file,
            Err(e) => {
                eprint!("could not open {:?}", in_file);
                let _ = update_channel.try_send(Err(Notification {
                    severity: Severity::WARNING,
                    content: format!("could not open file ({})", e),
                    line: None,
                }));
                return Err(err_msg(format!("could not open file ({})", e)));
            }
        };
        let reader =
            ReduxReader::with_capacity(10 * 1024 * 1024, f).set_policy(MinBuffered(10 * 1024));
        Ok(FileMessageProducer {
            reader,
            filter_config,
            index,
            update_channel,
            with_storage_header,
        })
    }
}
impl FileMessageProducer {
    fn produce_next_message(
        &mut self,
        fibex_metadata: Option<Rc<FibexMetadata>>,
    ) -> (usize, Result<Option<Message>, DltParseError>) {
        #[allow(clippy::never_loop)]
        let res = loop {
            match self.reader.fill_buf() {
                Ok(content) => {
                    trace!("got content: {} bytes", content.len());
                    if content.is_empty() {
                        return (0, Ok(None));
                    }
                    let available = content.len();

                    let res: nom::IResult<&[u8], Option<Message>> = dlt_message(
                        content,
                        self.filter_config.as_ref(),
                        self.index,
                        Some(&self.update_channel),
                        fibex_metadata,
                        self.with_storage_header,
                    );
                    match res {
                        Ok(r) => {
                            let consumed = available - r.0.len();
                            trace!("parse ok, consumed: {}", consumed);
                            break (consumed, Ok(r.1));
                        }
                        Err(nom::Err::Incomplete(n)) => {
                            trace!("parse incomplete");
                            let needed = match n {
                                nom::Needed::Size(s) => format!("{}", s),
                                nom::Needed::Unknown => "unknown".to_string(),
                            };
                            break (0, Err(DltParseError::Unrecoverable {
                            cause: format!(
                            "read_one_dlt_message: imcomplete parsing error for dlt messages: (bytes left: {}, but needed: {})",
                            content.len(),
                            needed
                        ),
                        }));
                        }
                        Err(nom::Err::Error(_e)) => {
                            trace!("parse error");
                            break (
                                DLT_PATTERN_SIZE,
                                Err(DltParseError::ParsingHickup {
                                    reason: format!(
                                    "read_one_dlt_message: parsing error for dlt messages: {:?}",
                                    _e
                                ),
                                }),
                            );
                        }
                        Err(nom::Err::Failure(_e)) => {
                            trace!("parse failure");
                            break (
                                0,
                                Err(DltParseError::Unrecoverable {
                                    cause: format!(
                                    "read_one_dlt_message: parsing failure for dlt messages: {:?}",
                                    _e
                                ),
                                }),
                            );
                        }
                    }
                }
                Err(e) => {
                    trace!("no more content");
                    break (
                        0,
                        Err(DltParseError::Unrecoverable {
                            cause: format!("error for filling buffer with dlt messages: {:?}", e),
                        }),
                    );
                }
            }
        };
        self.reader.consume(res.0);
        res
    }
}
// pub trait MessageProducer {
//     fn produce_next_message(
//         &mut self,
//         fibex_metadata: Option<Rc<FibexMetadata>>,
//     ) -> (usize, Result<Option<Message>, DltParseError>);
// }
#[allow(clippy::too_many_arguments)]
pub fn create_index_and_mapping_dlt_from_socket(
    socket_config: SocketConfig,
    tag: &str,
    ecu_id: String,
    out_path: &std::path::PathBuf,
    dlt_filter: Option<filtering::DltFilterConfig>,
    update_channel: &cc::Sender<ChunkResults>,
    shutdown_receiver: async_std::sync::Receiver<()>,
    fibex_metadata: Option<Rc<FibexMetadata>>,
) -> Result<(), Error> {
    trace!("create_index_and_mapping_dlt_from_socket");
    let res = match utils::next_line_nr(out_path) {
        Ok(initial_line_nr) => {
            let filter_config: Option<filtering::ProcessedDltFilterConfig> =
                dlt_filter.map(filtering::process_filter_config);
            match index_from_socket(
                socket_config,
                filter_config,
                update_channel.clone(),
                fibex_metadata,
                tag,
                ecu_id,
                out_path,
                initial_line_nr,
                shutdown_receiver,
            ) {
                Err(ConnectionError::WrongConfiguration { cause }) => {
                    let _ = update_channel.send(Err(Notification {
                        severity: Severity::ERROR,
                        content: cause.clone(),
                        line: None,
                    }));
                    Err(err_msg(cause))
                }
                Err(ConnectionError::UnableToConnect { reason }) => {
                    let _ = update_channel.send(Err(Notification {
                        severity: Severity::ERROR,
                        content: reason.clone(),
                        line: None,
                    }));
                    Err(err_msg(reason))
                }
                Err(ConnectionError::Other { info }) => {
                    let _ = update_channel.send(Err(Notification {
                        severity: Severity::ERROR,
                        content: info.clone(),
                        line: None,
                    }));
                    Err(err_msg(info))
                }
                Ok(_) => Ok(()),
            }
        }
        Err(e) => {
            let content = format!(
                "could not determine last line number of {:?} ({})",
                out_path, e
            );
            let _ = update_channel.send(Err(Notification {
                severity: Severity::ERROR,
                content: content.clone(),
                line: None,
            }));
            Err(err_msg(content))
        }
    };
    let _ = update_channel.send(Ok(IndexingProgress::Finished));
    res
}
pub fn create_index_and_mapping_dlt(
    config: IndexingConfig,
    source_file_size: Option<usize>,
    dlt_filter: Option<filtering::DltFilterConfig>,
    update_channel: &cc::Sender<ChunkResults>,
    shutdown_receiver: Option<cc::Receiver<()>>,
    fibex_metadata: Option<Rc<FibexMetadata>>,
) -> Result<(), Error> {
    trace!("create_index_and_mapping_dlt");
    match utils::next_line_nr(config.out_path) {
        Ok(initial_line_nr) => {
            let filter_config: Option<filtering::ProcessedDltFilterConfig> =
                dlt_filter.map(filtering::process_filter_config);
            let mut message_producer = FileMessageProducer::new(
                &config.in_file,
                filter_config,
                initial_line_nr,
                update_channel.clone(),
                true,
            )?;
            index_dlt_content(
                config,
                initial_line_nr,
                source_file_size,
                update_channel,
                shutdown_receiver,
                fibex_metadata,
                &mut message_producer,
            )
        }
        Err(e) => {
            let content = format!(
                "could not determine last line number of {:?} ({})",
                config.out_path, e
            );
            let _ = update_channel.send(Err(Notification {
                severity: Severity::ERROR,
                content: content.clone(),
                line: None,
            }));
            Err(err_msg(content))
        }
    }
}

/// create index for a dlt file
/// source_file_size: if progress updates should be made, add this value
pub fn index_dlt_content(
    config: IndexingConfig,
    initial_line_nr: usize,
    source_file_size: Option<usize>,
    update_channel: &cc::Sender<ChunkResults>,
    shutdown_receiver: Option<cc::Receiver<()>>,
    fibex_metadata: Option<Rc<FibexMetadata>>,
    message_producer: &mut FileMessageProducer,
) -> Result<(), Error> {
    trace!("index_dlt_file {:?}", config);
    let (out_file, current_out_file_size) =
        utils::get_out_file_and_size(config.append, &config.out_path)?;

    let mut chunk_count = 0usize;
    let mut last_byte_index = 0usize;
    let mut chunk_factory = ChunkFactory::new(config.chunk_size, current_out_file_size);
    let mut line_nr = initial_line_nr;
    let mut buf_writer = BufWriter::with_capacity(10 * 1024 * 1024, out_file);

    let mut progress_reporter = ProgressReporter::new(source_file_size, update_channel.clone());

    let mut stopped = false;
    loop {
        if stopped {
            info!("we were stopped in dlt-indexer",);
            break;
        };
        let (consumed, next) = message_producer.produce_next_message(fibex_metadata.clone());
        if consumed == 0 {
            break;
        } else {
            progress_reporter.make_progress(consumed);
        }
        match next {
            Ok(Some(msg)) => {
                trace!("next was Ok(msg){} bytes", msg.as_bytes().len());
                let written_bytes_len =
                    utils::create_tagged_line_d(config.tag, &mut buf_writer, &msg, line_nr, true)?;
                line_nr += 1;
                if let Some(chunk) =
                    chunk_factory.create_chunk_if_needed(line_nr, written_bytes_len)
                {
                    // check if stop was requested
                    if let Some(rx) = shutdown_receiver.as_ref() {
                        match rx.try_recv() {
                            // Shutdown if we have received a command or if there is
                            // nothing to send it.
                            Ok(_) | Err(cc::TryRecvError::Disconnected) => {
                                info!("shutdown received in indexer",);
                                stopped = true // stop
                            }
                            // No shutdown command, continue
                            Err(cc::TryRecvError::Empty) => (),
                        }
                    };
                    chunk_count += 1;
                    last_byte_index = chunk.b.1;
                    update_channel.send(Ok(IndexingProgress::GotItem { item: chunk }))?;
                    buf_writer.flush()?;
                }
            }
            Ok(None) => {
                trace!("next was Ok(None)");
            }
            Err(e) => match e {
                DltParseError::ParsingHickup { reason } => {
                    trace!(
                        "error parsing 1 dlt message, try to continue parsing: {}",
                        reason
                    );
                }
                DltParseError::Unrecoverable { cause } => {
                    warn!("cannot continue parsing: {}", cause);
                    update_channel.send(Err(Notification {
                        severity: Severity::ERROR,
                        content: format!("error parsing dlt file: {}", cause),
                        line: None,
                    }))?;
                    break;
                }
            },
        }
    }

    buf_writer.flush()?;
    if let Some(chunk) = chunk_factory.create_last_chunk(line_nr, chunk_count == 0) {
        update_channel.send(Ok(IndexingProgress::GotItem {
            item: chunk.clone(),
        }))?;
        chunk_count += 1;
        last_byte_index = chunk.b.1;
    }
    if chunk_count > 0 {
        let last_expected_byte_index = fs::metadata(config.out_path).map(|md| md.len() as usize)?;
        if last_expected_byte_index != last_byte_index {
            update_channel.send(Err(Notification {
                severity: Severity::ERROR,
                content: format!(
                    "error in computation! last byte in chunks is {} but should be {}",
                    last_byte_index, last_expected_byte_index
                ),
                line: Some(line_nr),
            }))?;
        }
    }
    trace!("sending IndexingProgress::Finished");
    update_channel.send(Ok(IndexingProgress::Finished))?;
    Ok(())
}

#[derive(Serialize, Debug, Default)]
struct LevelDistribution {
    non_log: usize,
    log_fatal: usize,
    log_error: usize,
    log_warning: usize,
    log_info: usize,
    log_debug: usize,
    log_verbose: usize,
    log_invalid: usize,
}
impl LevelDistribution {
    pub fn new(level: Option<LogLevel>) -> LevelDistribution {
        let all_zero = Default::default();
        match level {
            None => LevelDistribution {
                non_log: 1,
                ..all_zero
            },
            Some(LogLevel::Fatal) => LevelDistribution {
                log_fatal: 1,
                ..all_zero
            },
            Some(LogLevel::Error) => LevelDistribution {
                log_error: 1,
                ..all_zero
            },
            Some(LogLevel::Warn) => LevelDistribution {
                log_warning: 1,
                ..all_zero
            },
            Some(LogLevel::Info) => LevelDistribution {
                log_info: 1,
                ..all_zero
            },
            Some(LogLevel::Debug) => LevelDistribution {
                log_debug: 1,
                ..all_zero
            },
            Some(LogLevel::Verbose) => LevelDistribution {
                log_verbose: 1,
                ..all_zero
            },
            _ => LevelDistribution {
                log_invalid: 1,
                ..all_zero
            },
        }
    }
}
type IdMap = FxHashMap<String, LevelDistribution>;

fn add_for_level(level: Option<LogLevel>, ids: &mut IdMap, id: String) {
    if let Some(n) = ids.get_mut(&id) {
        match level {
            Some(LogLevel::Fatal) => {
                *n = LevelDistribution {
                    log_fatal: n.log_fatal + 1,
                    ..*n
                }
            }
            Some(LogLevel::Error) => {
                *n = LevelDistribution {
                    log_error: n.log_error + 1,
                    ..*n
                }
            }
            Some(LogLevel::Warn) => {
                *n = LevelDistribution {
                    log_warning: n.log_warning + 1,
                    ..*n
                }
            }
            Some(LogLevel::Info) => {
                *n = LevelDistribution {
                    log_info: n.log_info + 1,
                    ..*n
                }
            }
            Some(LogLevel::Debug) => {
                *n = LevelDistribution {
                    log_debug: n.log_debug + 1,
                    ..*n
                };
            }
            Some(LogLevel::Verbose) => {
                *n = LevelDistribution {
                    log_verbose: n.log_verbose + 1,
                    ..*n
                };
            }
            Some(LogLevel::Invalid(_)) => {
                *n = LevelDistribution {
                    log_invalid: n.log_invalid + 1,
                    ..*n
                };
            }
            None => {
                *n = LevelDistribution {
                    non_log: n.non_log + 1,
                    ..*n
                };
            }
        }
    } else {
        ids.insert(id, LevelDistribution::new(level));
    }
}
#[derive(Serialize, Debug)]
pub struct StatisticInfo {
    app_ids: Vec<(String, LevelDistribution)>,
    context_ids: Vec<(String, LevelDistribution)>,
    ecu_ids: Vec<(String, LevelDistribution)>,
    contained_non_verbose: bool,
}
pub type StatisticsResults = std::result::Result<IndexingProgress<StatisticInfo>, Notification>;
pub fn get_dlt_file_info(
    in_file: &std::path::PathBuf,
    update_channel: &cc::Sender<StatisticsResults>,
    shutdown_receiver: Option<cc::Receiver<()>>,
) -> Result<(), Error> {
    let f = match fs::File::open(in_file) {
        Ok(file) => file,
        Err(e) => {
            error!("could not open {:?}", in_file);
            return Err(err_msg(format!("could not open {:?} ({})", in_file, e)));
        }
    };

    let source_file_size: usize = fs::metadata(&in_file)?.len() as usize;
    let mut reader =
        ReduxReader::with_capacity(10 * 1024 * 1024, f).set_policy(MinBuffered(10 * 1024));

    let mut app_ids: IdMap = FxHashMap::default();
    let mut context_ids: IdMap = FxHashMap::default();
    let mut ecu_ids: IdMap = FxHashMap::default();
    let mut index = 0usize;
    let mut processed_bytes = 0usize;
    let mut contained_non_verbose = false;
    loop {
        match read_one_dlt_message_info(&mut reader, Some(index), Some(update_channel)) {
            Ok(Some((
                consumed,
                StatisticRowInfo {
                    app_id_context_id: Some((app_id, context_id)),
                    ecu_id: ecu,
                    level,
                    verbose,
                },
            ))) => {
                contained_non_verbose = contained_non_verbose || !verbose;
                reader.consume(consumed);
                add_for_level(level, &mut app_ids, app_id);
                add_for_level(level, &mut context_ids, context_id);
                match ecu {
                    Some(id) => add_for_level(level, &mut ecu_ids, id),
                    None => add_for_level(level, &mut ecu_ids, "NONE".to_string()),
                };
                processed_bytes += consumed;
            }
            Ok(Some((
                consumed,
                StatisticRowInfo {
                    app_id_context_id: None,
                    ecu_id: ecu,
                    level,
                    verbose,
                },
            ))) => {
                contained_non_verbose = contained_non_verbose || !verbose;
                reader.consume(consumed);
                add_for_level(level, &mut app_ids, "NONE".to_string());
                add_for_level(level, &mut context_ids, "NONE".to_string());
                match ecu {
                    Some(id) => add_for_level(level, &mut ecu_ids, id),
                    None => add_for_level(level, &mut ecu_ids, "NONE".to_string()),
                };
                processed_bytes += consumed;
            }
            Ok(None) => {
                break;
            }
            // Err(e) => {
            //     return Err(err_msg(format!(
            //         "error while parsing dlt messages[{}]: {}",
            //         index, e
            //     )))
            Err(e) => {
                // we couldn't parse the message. try to skip it and find the next.
                trace!("stats...try to skip and continue parsing: {}", e);
                match e {
                    DltParseError::ParsingHickup { reason } => {
                        // we couldn't parse the message. try to skip it and find the next.
                        reader.consume(4); // at least skip the magic DLT pattern
                        trace!(
                            "error parsing 1 dlt message, try to continue parsing: {}",
                            reason
                        );
                    }
                    DltParseError::Unrecoverable { cause } => {
                        warn!("cannot continue parsing: {}", cause);
                        update_channel.send(Err(Notification {
                            severity: Severity::ERROR,
                            content: format!("error parsing dlt file: {}", cause),
                            line: None,
                        }))?;
                        break;
                    }
                }
            }
        }
        index += 1;
        if index % STOP_CHECK_LINE_THRESHOLD == 0 {
            // check if stop was requested
            if let Some(rx) = shutdown_receiver.as_ref() {
                match rx.try_recv() {
                    // Shutdown if we have received a command or if there is
                    // nothing to send it.
                    Ok(_) | Err(cc::TryRecvError::Disconnected) => {
                        info!("shutdown received in dlt stats producer, sending stopped");
                        update_channel.send(Ok(IndexingProgress::Stopped))?;
                        break;
                    }
                    // No shutdown command, continue
                    Err(cc::TryRecvError::Empty) => (),
                }
            };
            update_channel.send(Ok(IndexingProgress::Progress {
                ticks: (processed_bytes, source_file_size),
            }))?;
        }
    }
    let res = StatisticInfo {
        app_ids: app_ids
            .into_iter()
            .collect::<Vec<(String, LevelDistribution)>>(),
        context_ids: context_ids
            .into_iter()
            .collect::<Vec<(String, LevelDistribution)>>(),
        ecu_ids: ecu_ids
            .into_iter()
            .collect::<Vec<(String, LevelDistribution)>>(),
        contained_non_verbose,
    };

    update_channel.send(Ok(IndexingProgress::GotItem { item: res }))?;
    update_channel.send(Ok(IndexingProgress::Finished))?;
    Ok(())
}

#[derive(Serialize, Debug)]
pub struct StatisticRowInfo {
    app_id_context_id: Option<(String, String)>,
    ecu_id: Option<String>,
    level: Option<LogLevel>,
    verbose: bool,
}
fn read_one_dlt_message_info<T: Read>(
    reader: &mut ReduxReader<T, MinBuffered>,
    index: Option<usize>,
    update_channel: Option<&cc::Sender<StatisticsResults>>,
) -> Result<Option<(usize, StatisticRowInfo)>, DltParseError> {
    match reader.fill_buf() {
        Ok(content) => {
            if content.is_empty() {
                return Ok(None);
            }
            let available = content.len();
            let res: nom::IResult<&[u8], StatisticRowInfo> =
                dlt_statistic_row_info(content, index, update_channel);
            // println!("dlt statistic_row_info got: {:?}", res);
            match res {
                Ok(r) => {
                    let consumed = available - r.0.len();
                    Ok(Some((consumed, r.1)))
                }
                e => match e {
                    Err(nom::Err::Incomplete(n)) => {
                        trace!("parse incomplete");
                        let needed = match n {
                            nom::Needed::Size(s) => format!("{}", s),
                            nom::Needed::Unknown => "unknown".to_string(),
                        };
                        Err(DltParseError::Unrecoverable {
                            cause: format!(
                            "read_one_dlt_message: imcomplete parsing error for dlt messages: (bytes left: {}, but needed: {})",
                            content.len(),
                            needed
                        ),
                        })
                    }
                    Err(nom::Err::Error(_e)) => Err(DltParseError::ParsingHickup {
                        reason: format!("parsing error for dlt message info: {:?}", _e),
                    }),
                    Err(nom::Err::Failure(_e)) => Err(DltParseError::Unrecoverable {
                        cause: format!("parsing failure for dlt message infos: {:?}", _e),
                    }),
                    _ => Err(DltParseError::Unrecoverable {
                        cause: format!("error while parsing dlt message infos: {:?}", e),
                    }),
                },
            }
        }
        Err(e) => Err(DltParseError::ParsingHickup {
            reason: format!("error while parsing dlt messages: {}", e),
        }),
    }
}
