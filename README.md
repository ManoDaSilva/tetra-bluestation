```
░▀█▀░█▀▀░▀█▀░█▀▄░█▀█░░░░░█▀▄░█░░░█░█░█▀▀░█▀▀░▀█▀░█▀█░▀█▀░▀█▀░█▀█░█▀█
░░█░░█▀▀░░█░░█▀▄░█▀█░▄▄▄░█▀▄░█░░░█░█░█▀▀░▀▀█░░█░░█▀█░░█░░░█░░█░█░█░█
░░▀░░▀▀▀░░▀░░▀░▀░▀░▀░░░░░▀▀░░▀▀▀░▀▀▀░▀▀▀░▀▀▀░░▀░░▀░▀░░▀░░▀▀▀░▀▀▀░▀░▀
```

This is a FOSS TETRA stack aimed at providing an extensible basis for TETRA experimentation and research. At this point, it's alpha code. The stack serves a downlink base station signal, and a properly configured MS is able to receive the emitted downlink signal, connect to it, and attach to talkgroups. Voice calls are partially supported. Connectivity through Brew with the larger BrandMeister network is also optionally available. Lots of other functionality is currently not implemented, although parsing code for most TETRA protocol messages is already present. 

## Documentation

Project documentation for tetra-bluestation is maintained in a separate repository, as a wiki.

https://github.com/MidnightBlueLabs/tetra-bluestation-docs/wiki

The documentation repository contains:
- Hardware and SDR considerations 
- Configuration file reference and examples  
- Build and runtime instructions   
- Practical notes 

Contributions to the documentation follow the same pull-request-based workflow as the main codebase, see the appropriate "Contributions" chapter.

## BS T1 test mode

For receiver validation with a TETRA radio test set (tested with an IFR 2968 in BS T1 mode), set `stack_mode = "BsT1"`. The stack then runs only the PHY and a test entity: no Brew, telemetry, control or signalling entities. See `example_config/config_t1.toml`.

- The base station transmits the sync burst (BSCH and BNCH/T, announcing T1 channel type 7) in frame 18, slot 1, and a PRBS TCH/7,2 signal in all other slots (EN 300 394-1, clause 9.2.2).
- The test set sends T1 type 7 (TCH/7,2, ITU-T O.153 PRBS-511) on the uplink. The stack compares every received burst with the PRBS and reports bit errors once per interval.
- The report line shows detected and received bursts, the bit error ratio over the whole run and over the last interval, the share of bursts with errors, an errors-per-burst histogram, and the split of errors between the two halves of a burst. A burst is counted as received when it matches the PRBS with at most 30 % bit errors, so very weak signals show up as missing bursts instead of in the BER.
- With `ber_limit_percent` and `min_bits` set (EN 300 394-1 Table A.5 lists values per test case), the report also shows how far the measurement has settled and a PASS or FAIL verdict.
- `output = "json"` prints one JSON object per line, for scripts. The last line has `"final": true`. The process exit code is 0 for pass, 1 for fail and 2 when no usable measurement was made (no bits, or fewer than `min_bits`). Set `duration_s` to make a run end by itself.
- In this mode the T1 reports go to stdout and the log goes to stderr at `warn` level. Use `RUST_LOG=info` (or `debug`) for more log output. The banner and the SDR driver messages also go to stderr, so `2>/dev/null` leaves only the T1 lines.
- On the test set, use SYNC TO BASE STATION = AUTO and MCC-MNC-BCC UPDATE = AUTO, set the Rx offset to the duplex spacing of the cell, and BS Tx mode to CONTINUOUS ALL SLOTS.

## Acknowledgements

- Thanks to Harald Welte and the osmocom crew for their amazing initial work on osmocom-tetra, without which this project would not have existed. 
- Many thanks to Tatu Peltola, who graciously augmented rust-soapysdr with the required timestamping functionality to facilitate robust rx/tx, and also provided a rust-native Viterbi encoder/decoder class used in the LMAC.
- Many thanks to the awesome contributers helping to make BlueStation as stable, fancy and feature-rich as can be. 
- Thanks to Stichting NLnet, who agreed on allocating a part of the [RETETRA3 project](https://nlnet.nl/project/RETETRA3/) grant to the implementation of FOSS software for TETRA. 
