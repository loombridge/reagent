//! Will hold Reagent as a plug-in (CLAP, VST3 and Audio Unit) and as a standalone app, through the plug-in kit's format layer.

// The plug-in joins the engine and the interface through the kit's format
// layer; this keeps the dependencies linked until the first code uses them.
use lspk_plugin as _;
use reagent_engine as _;
use reagent_ui as _;
