// HTDemucs stores each output channel's coefficients in one contiguous row.
// The workgroup reads two 16x16 tiles and produces a 16x16 output tile.
struct Shape {
    rows: u32,
    inner: u32,
    cols: u32,
    first_row: u32,
}

@group(0) @binding(0) var<storage, read> input_values: array<f32>;
@group(0) @binding(1) var<storage, read> weights: array<f32>;
@group(0) @binding(2) var<storage, read_write> output_values: array<f32>;
@group(0) @binding(3) var<uniform> shape: Shape;

var<workgroup> inputs: array<array<f32, 16>, 16>;
// Read coefficients across adjacent output channels. Padding avoids bank conflicts on tile loads.
var<workgroup> coefficients: array<array<f32, 17>, 16>;

@compute @workgroup_size(8, 8)
fn linear(@builtin(workgroup_id) group: vec3<u32>, @builtin(local_invocation_id) lane: vec3<u32>) {
    let first_row = shape.first_row + group.y * 16u;
    let first_col = group.x * 16u;
    var acc = vec4<f32>(0.0);
    for (var start = 0u; start < shape.inner; start += 16u) {
        // Consecutive lanes read consecutive channels from both source buffers.
        for (var offset = lane.y * 8u + lane.x; offset < 256u; offset += 64u) {
            let i = offset / 16u;
            let j = offset % 16u;
            let k = start + j;
            inputs[i][j] = 0.0;
            coefficients[j][i] = 0.0;
            if (first_row + i < shape.rows && k < shape.inner) {
                inputs[i][j] = input_values[(first_row + i) * shape.inner + k];
            }
            if (first_col + i < shape.cols && k < shape.inner) {
                coefficients[j][i] = weights[(first_col + i) * shape.inner + k];
            }
        }
        workgroupBarrier();
        for (var k = 0u; k < 16u; k++) {
            let a = inputs[lane.y][k];
            let b = inputs[lane.y + 8u][k];
            let c = coefficients[k][lane.x];
            let d = coefficients[k][lane.x + 8u];
            acc += vec4<f32>(a * c, a * d, b * c, b * d);
        }
        workgroupBarrier();
    }

    let row = first_row + lane.y;
    let col = first_col + lane.x;
    if (row < shape.rows && col < shape.cols) {
        output_values[row * shape.cols + col] = acc.x;
    }
    if (row < shape.rows && col + 8u < shape.cols) {
        output_values[row * shape.cols + col + 8u] = acc.y;
    }
    if (row + 8u < shape.rows && col < shape.cols) {
        output_values[(row + 8u) * shape.cols + col] = acc.z;
    }
    if (row + 8u < shape.rows && col + 8u < shape.cols) {
        output_values[(row + 8u) * shape.cols + col + 8u] = acc.w;
    }
}
