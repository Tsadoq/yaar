// For the tests only: a stand-in for `@earendil-works/pi-ai`, the module of pi's that the
// app's extension imports at run time for `Type`, the builder of a tool's parameters. The
// test of the extension puts it where Node resolves that name from the extension's file,
// as `node_modules/@earendil-works/pi-ai/index.mjs`. Each builder gives the plain JSON
// Schema object it stands for: `Type.Object(properties)` an object schema whose `required`
// lists every property not made with `Type.Optional`; `Type.String(options)` and
// `Type.Boolean(options)` the schema of their type with the options' keys; and
// `Type.Optional(schema)` the schema marked optional.
//
// Not built yet: the package of pi's extension writes it.

export const Type = {};
