# Graph Canvas and Layer Coordinate Contract

This document records the coordinate behavior used by graph evaluation and
the Viewer canvas. It is a compatibility reference for preview scaling,
compositing, and transform changes.

## Document canvas

- The document canvas has a positive integer width and height in image pixels.
- Its origin is the top-left; positive X moves right and positive Y moves down.
- Generators and layer containers use this size as their output bounds.
- The Viewer may display a scaled copy of the document, but display zoom does
  not change graph coordinates or saved transforms.

## Layer transforms

- `ViewerLayer.center` is the layer center in document canvas coordinates.
- `ViewerLayer.size` is the unrotated layer size in canvas units.
- `ViewerLayer.angle` is radians, clockwise on screen.
- A layer's source image dimensions describe its pixels; they do not define the
  document canvas or override an explicit saved transform.
- An unset transform is eligible for initial fit-to-canvas behavior in the
  Viewer/compositor. A saved non-zero transform is authoritative and is kept
  when a source preview or output changes.

## Render resolution

- A render target's pixel dimensions are independent of a layer's source pixel
  dimensions and its canvas-space transform.
- When a render target is scaled for a preview, layer centers and sizes are
  multiplied by the same scale factor. Angles do not change.
- Route nodes forward their source and do not create a coordinate space.
- Output nodes expose the document canvas size; groups and blends use their
  stored canvas size when set, with the graph canvas as fallback.

Changes to these rules require a regression case for both output dimensions
and the affected layer transform, plus a note about saved-session behavior.
