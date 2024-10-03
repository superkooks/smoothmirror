#version 410

uniform sampler2D frag_tex;

in vec2 texcoords;

out vec4 color;

void main() {
    color = texture(frag_tex, texcoords);
}
