//! Downlink generation for BS T1 test mode.
//!
//! EN 300 394-1 clause 9.2.2: the BS transmits a synchronization burst with BSCH and BNCH/T in
//! frame 18, slot 1, and a channel type 1 signal (TCH/7,2) in all other slots. The BNCH/T tells
//! the test system which uplink channel type the BS expects (channel type 7, TCH/7,2).

use tetra_config::bluestation::StackConfig;
use tetra_core::freqs::FreqInfo;
use tetra_core::{BitBuffer, BurstType, TdmaTime, TrainingSequence};
use tetra_pdus::mle::pdus::d_mle_sync::DMleSync;
use tetra_pdus::umac::enums::access_assign_dl_usage::AccessAssignDlUsage;
use tetra_pdus::umac::enums::access_assign_ul_usage::AccessAssignUlUsage;
use tetra_pdus::umac::enums::access_code::AccessCode;
use tetra_pdus::umac::pdus::access_assign::AccessAssign;
use tetra_pdus::umac::pdus::access_assign_fr18::AccessAssignFr18;
use tetra_pdus::umac::pdus::mac_sync::MacSync;
use tetra_pdus::umac::structs::access_field::AccessField;
use tetra_pdus::umac::structs::base_frame_length::BaseFrameLength;
use tetra_saps::tmv::TmvUnitdataReq;
use tetra_saps::tmv::enums::logical_chans::LogicalChannel;
use tetra_saps::tp::TpUnitdataReqSlot;

use crate::lmac::components::{errorcontrol, scrambler};
use crate::t1test::prbs::Prbs511;

/// Table A.20, T1_T4_burst_type: channel type 7 is TCH/7,2 on the uplink
pub const T1_BURST_TYPE_TCH72_UL: u8 = 7;

const TCH72_BLOCK_BITS: usize = 432;
const BNCH_T_BITS: usize = 124;
const BSCH_BITS: usize = 60;

/// Parameters of the cell, taken from the stack config
#[derive(Debug, Clone)]
pub struct T1DlParams {
    pub mcc: u16,
    pub mnc: u16,
    pub colour_code: u8,
    pub system_code: u8,
    pub sharing_mode: u8,
    pub ts_reserved_frames: u8,
    pub u_plane_dtx: bool,
    pub frame_18_ext: bool,
    pub late_entry_supported: bool,
    pub main_carrier: u16,
    pub freq_band: u8,
    pub freq_offset_index: u8,
    pub duplex_spacing: u8,
    pub reverse_operation: bool,
}

impl T1DlParams {
    pub fn from_config(c: &StackConfig) -> Self {
        Self {
            mcc: c.net.mcc,
            mnc: c.net.mnc,
            colour_code: c.cell.colour_code,
            system_code: c.cell.system_code,
            sharing_mode: c.cell.sharing_mode,
            ts_reserved_frames: c.cell.ts_reserved_frames,
            u_plane_dtx: c.cell.u_plane_dtx,
            frame_18_ext: c.cell.frame_18_ext,
            late_entry_supported: c.cell.late_entry_supported,
            main_carrier: c.cell.main_carrier,
            freq_band: c.cell.freq_band,
            freq_offset_index: FreqInfo::freq_offset_hz_to_id(c.cell.freq_offset_hz).expect("invalid freq_offset"),
            duplex_spacing: c.cell.duplex_spacing_id,
            reverse_operation: c.cell.reverse_operation,
        }
    }
}

/// Generates the downlink slots of the T1 test signal
pub struct T1DlGen {
    params: T1DlParams,
    scrambling_code: u32,
    prbs: Prbs511,
}

impl T1DlGen {
    pub fn new(params: T1DlParams) -> Self {
        let scrambling_code = scrambler::tetra_scramb_get_init(params.mcc, params.mnc, params.colour_code);
        Self {
            params,
            scrambling_code,
            prbs: Prbs511::new(),
        }
    }

    pub fn scrambling_code(&self) -> u32 {
        self.scrambling_code
    }

    /// Builds the slot to transmit at time `ts`
    pub fn build_slot(&mut self, ts: TdmaTime) -> TpUnitdataReqSlot {
        if ts.f == 18 && ts.t == 1 {
            self.build_sync_burst(ts)
        } else {
            self.build_tch72_burst(ts)
        }
    }

    /// Synchronization burst: BSCH in block 1, BNCH/T in block 2
    fn build_sync_burst(&self, ts: TdmaTime) -> TpUnitdataReqSlot {
        let p = &self.params;

        let mac_sync = MacSync {
            system_code: p.system_code,
            colour_code: p.colour_code,
            time: ts,
            sharing_mode: p.sharing_mode,
            ts_reserved_frames: p.ts_reserved_frames,
            u_plane_dtx: p.u_plane_dtx,
            frame_18_ext: p.frame_18_ext,
        };
        let mle_sync = DMleSync {
            mcc: p.mcc,
            mnc: p.mnc,
            neighbor_cell_broadcast: 0,
            cell_load_ca: 0,
            late_entry_supported: p.late_entry_supported,
        };
        let mut bsch = BitBuffer::new(BSCH_BITS);
        mac_sync.to_bitbuf(&mut bsch);
        mle_sync.to_bitbuf(&mut bsch);

        let blk1 = errorcontrol::encode_cp(TmvUnitdataReq {
            mac_block: bsch,
            logical_channel: LogicalChannel::Bsch,
            scrambling_code: scrambler::SCRAMB_INIT,
        });
        let blk2 = errorcontrol::encode_cp(TmvUnitdataReq {
            mac_block: self.build_bnch_t(),
            logical_channel: LogicalChannel::Bnch,
            scrambling_code: self.scrambling_code,
        });

        TpUnitdataReqSlot {
            train_type: TrainingSequence::SyncTrainSeq,
            burst_type: BurstType::SDB,
            bbk: Some(self.build_aach(ts)),
            blk1: Some(blk1),
            blk2: Some(blk2),
        }
    }

    /// BNCH/T content, Table A.20 of EN 300 394-1. 124 bits.
    /// Reception test: Tx_on 0, expecting channel type 7 on the uplink, no loopback.
    pub fn build_bnch_t(&self) -> BitBuffer {
        let p = &self.params;
        let mut b = BitBuffer::new(BNCH_T_BITS);
        b.write_bits(0b10, 2); // PDU type: broadcast
        b.write_bits(0b00, 2); // Broadcast type: SYSINFO
        b.write_bits(p.main_carrier as u64, 12);
        b.write_bits(p.freq_band as u64, 4);
        b.write_bits(p.freq_offset_index as u64, 2);
        b.write_bits(p.duplex_spacing as u64, 3);
        b.write_bits(p.reverse_operation as u64, 1);
        b.write_bits(0, 2); // No of common secondary control channels
        b.write_bits(0b001, 3); // MS_TXPWR_MAX_CELL: 15 dBm
        b.write_bits(0, 4); // RXLEV_ACCESS_MIN
        b.write_bits(0, 4); // ACCESS_PARAMETER: -53 dBm
        b.write_bits(0, 4); // RADIO_DOWNLINK_TIMEOUT: disabled
        b.write_bits(0, 1); // Tx_on: reception on
        b.write_bits(0, 1); // Tx_burst_type: normal uplink burst
        b.write_bits(T1_BURST_TYPE_TCH72_UL as u64, 5); // T1_T4_burst_type
        b.write_bits(0, 1); // Loop_back: off
        b.write_bits(0, 1); // Error correction: on (only used for QAM)
        b.write_bits(0, 5); // Extended burst type
        b.write_bits(0, 30); // Reserved
        b.write_bits(0, 5); // QAM_payload_type
        b.write_bits(0, 3); // Carrier bandwidth
        b.write_bits(0, 1); // Test signal width
        b.write_bits(0, 2); // PRBS continuation: undefined
        b.write_bits(0, 26); // Reserved
        assert_eq!(b.get_len(), BNCH_T_BITS);
        b.seek(0);
        b
    }

    /// Channel type 1: a full-slot TCH/7,2 normal burst carrying PRBS data.
    /// TCH/7,2 is unprotected: the 432 type-1 bits are only scrambled (EN 300 392-2, clause 8.3.1.3.4).
    fn build_tch72_burst(&mut self, ts: TdmaTime) -> TpUnitdataReqSlot {
        let mut bits = [0u8; TCH72_BLOCK_BITS];
        self.prbs.fill(&mut bits);
        let mut blk = BitBuffer::from_bitarr(&bits);
        scrambler::tetra_scramb_bits(self.scrambling_code, &mut blk);

        TpUnitdataReqSlot {
            train_type: TrainingSequence::NormalTrainSeq1,
            burst_type: BurstType::NDB,
            bbk: Some(self.build_aach(ts)),
            blk1: Some(blk),
            blk2: None,
        }
    }

    /// AACH in the same form the regular scheduler uses for an idle cell
    fn build_aach(&self, ts: TdmaTime) -> BitBuffer {
        let default_af = AccessField {
            access_code: AccessCode::AccessCodeA,
            base_frame_len: BaseFrameLength::Subslots2,
        };
        let mut buf = BitBuffer::new(14);
        if ts.f == 18 {
            AccessAssignFr18::UplinkCommonOnly {
                access_field_1: default_af,
                access_field_2: default_af,
            }
            .to_bitbuf(&mut buf);
        } else if ts.t == 1 {
            AccessAssign::DownlinkCommonControlUplinkCommonOnly {
                access_field_1: default_af,
                access_field_2: default_af,
            }
            .to_bitbuf(&mut buf);
        } else {
            AccessAssign::DownlinkDefinedUplinkDefined {
                downlink_usage_marker: AccessAssignDlUsage::Unallocated,
                uplink_usage_marker: AccessAssignUlUsage::Unallocated,
            }
            .to_bitbuf(&mut buf);
        }
        buf.seek(0);
        errorcontrol::encode_aach(buf, self.scrambling_code)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetra_core::{PhyBlockNum, PhyBlockType};
    use tetra_saps::tp::TpUnitdataInd;

    fn test_params() -> T1DlParams {
        T1DlParams {
            mcc: 204,
            mnc: 1337,
            colour_code: 1,
            system_code: 3,
            sharing_mode: 0,
            ts_reserved_frames: 0,
            u_plane_dtx: false,
            frame_18_ext: false,
            late_entry_supported: false,
            main_carrier: 1521,
            freq_band: 4,
            freq_offset_index: 0,
            duplex_spacing: 0,
            reverse_operation: false,
        }
    }

    fn ts(t: u8, f: u8) -> TdmaTime {
        TdmaTime { t, f, m: 7, h: 0 }
    }

    #[test]
    fn sync_burst_decodes_to_configured_cell() {
        let mut gen_ = T1DlGen::new(test_params());
        let slot = gen_.build_slot(ts(1, 18));
        assert_eq!(slot.burst_type, BurstType::SDB);
        assert_eq!(slot.train_type, TrainingSequence::SyncTrainSeq);

        // BSCH: descramble with the fixed BSCH code, parse MAC-SYNC and D-MLE-SYNC
        let ind = TpUnitdataInd {
            train_type: TrainingSequence::SyncTrainSeq,
            burst_type: BurstType::SDB,
            block_type: PhyBlockType::SB1,
            block_num: PhyBlockNum::Block1,
            block: slot.blk1.clone().unwrap(),
        };
        let (bits, crc_ok) = errorcontrol::decode_cp(LogicalChannel::Bsch, ind, None);
        assert!(crc_ok, "BSCH CRC");
        let mut bits = bits.unwrap();
        let sync = MacSync::from_bitbuf(&mut bits).unwrap();
        assert_eq!(sync.colour_code, 1);
        assert_eq!((sync.time.t, sync.time.f, sync.time.m), (1, 18, 7));
        let mle = DMleSync::from_bitbuf(&mut bits).unwrap();
        assert_eq!((mle.mcc, mle.mnc), (204, 1337));

        // BNCH/T: decodes with the cell scrambling code, announces channel type 7
        let ind = TpUnitdataInd {
            train_type: TrainingSequence::SyncTrainSeq,
            burst_type: BurstType::SDB,
            block_type: PhyBlockType::SB2,
            block_num: PhyBlockNum::Block2,
            block: slot.blk2.clone().unwrap(),
        };
        let (bits, crc_ok) = errorcontrol::decode_cp(LogicalChannel::Bnch, ind, Some(gen_.scrambling_code()));
        assert!(crc_ok, "BNCH/T CRC");
        let mut bits = bits.unwrap();
        assert_eq!(bits.read_bits(2).unwrap(), 0b10); // PDU type
        assert_eq!(bits.read_bits(2).unwrap(), 0b00); // Broadcast type
        assert_eq!(bits.read_bits(12).unwrap(), 1521); // Main carrier
        assert_eq!(bits.read_bits(4).unwrap(), 4); // Band
        bits.read_bits(2 + 3 + 1 + 2 + 3 + 4 + 4 + 4).unwrap(); // up to Tx_on
        assert_eq!(bits.read_bits(1).unwrap(), 0); // Tx_on
        assert_eq!(bits.read_bits(1).unwrap(), 0); // Tx_burst_type
        assert_eq!(bits.read_bits(5).unwrap(), T1_BURST_TYPE_TCH72_UL as u64);
    }

    #[test]
    fn other_slots_carry_scrambled_prbs() {
        let mut gen_ = T1DlGen::new(test_params());
        let slot = gen_.build_slot(ts(2, 5));
        assert_eq!(slot.burst_type, BurstType::NDB);
        assert!(slot.blk2.is_none());

        // Descramble and compare against a fresh PRBS
        let mut blk = slot.blk1.unwrap();
        assert_eq!(blk.get_len(), 432);
        scrambler::tetra_scramb_bits(gen_.scrambling_code(), &mut blk);
        let mut rx = [0u8; 432];
        blk.to_bitarr(&mut rx);
        let mut p = Prbs511::new();
        let mut expect = [0u8; 432];
        p.fill(&mut expect);
        assert_eq!(rx, expect);

        // Frame 18 slots 2-4 and slot 1 of other frames are also TCH/7,2
        assert_eq!(gen_.build_slot(ts(3, 18)).burst_type, BurstType::NDB);
        assert_eq!(gen_.build_slot(ts(1, 4)).burst_type, BurstType::NDB);
    }
}
