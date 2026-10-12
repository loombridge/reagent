//! Will hold Reagent's synthesis engine: the oscillators and their modes, sub and noise, the mixer and routing, the filters, the effects rack and buses, the modulation matrix's Reagent sources, the arpeggiator and clip sequencer, macros and global settings.

// The engine is built from the plug-in kit's DSP building blocks. Until the
// first engine code uses them, this keeps the dependency linked.
use lspk_dsp as _;
