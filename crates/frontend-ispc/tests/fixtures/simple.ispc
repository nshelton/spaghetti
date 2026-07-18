// Test fixture: exercises functions, structs, fields, calls, includes.
#include "helper.isph"

struct Sphere {
    float radius;
    float weight, bias;
};

static inline float square(float x)
{
    return x * x;
}

export void computeAreas(uniform float radii[], uniform float areas[], uniform int count)
{
    foreach (i = 0 ... count) {
        areas[i] = 4.0f * PI * square(radii[i]);
    }
}
