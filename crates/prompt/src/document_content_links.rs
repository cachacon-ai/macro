//! Rules for linking Macro items from content written *into* a Macro
//! document (via `CreateDocument`'s `fileContent`, or an `EditDocument`
//! `instructions` request for a mention/document-card), as distinct from the
//! model's own conversational reply text.
//!
//! `CreateDocument` content is parsed by the same Markdown pipeline used
//! everywhere else in Macro, so it must use the in-app `<m-document-mention>`
//! XML mention tags (see [`crate::mentions`]) to link to other Macro items —
//! never the plain Markdown links required for MCP chat replies (see
//! [`crate::mcp_item_links`]). That plain-link rule governs how the model
//! talks *to* an MCP client; it does not apply to content the model writes
//! *into* a Macro document, which the Macro app itself renders regardless of
//! which surface created it. This section is self-contained (it repeats the
//! document-mention tag shape) so it holds even where [`crate::mentions`] is
//! deliberately excluded, i.e. over MCP.

use crate::types::StaticPrompt;

static TITLE: &str = "Linking Macro items inside document content";

static INSTRUCTIONS: &str = r##"`CreateDocument` content (the `fileContent` argument) is rendered with the same Markdown parser used for chat responses inside the Macro app. Link to other Macro documents, channels, chats, projects, tasks, or email threads from within that content using `<m-document-mention>` XML mention tags, e.g.:

`<m-document-mention>{"documentId":"{id}","documentName":"","blockName":"md","blockParams":{}}</m-document-mention>`

This holds true even when `CreateDocument` is called through the MCP server, where you are otherwise told to link items in your own chat replies as plain Markdown URLs — that rule is about your conversational responses to the MCP client, not about content you write into a Macro document. Do NOT use plain Markdown links or bare URLs to reference other Macro items inside document content; only `<m-document-mention>` tags render as working links there.

The same applies to `EditDocument`: when its `instructions` ask for a mention or document-card, include the referenced item's id and name so the editing worker can construct the correct in-app markup itself.
"##;

static INTENT: &str = "Content written into Macro documents via CreateDocument or EditDocument \
links other Macro items with `<m-document-mention>` XML tags — never plain Markdown URLs — \
regardless of whether the tool call arrived in-app or over MCP.";

/// The document-content linking prompt.
pub static PROMPT: StaticPrompt<'static> = StaticPrompt::borrowed(TITLE, INSTRUCTIONS, INTENT);
