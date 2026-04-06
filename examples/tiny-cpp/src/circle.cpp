#include "circle.h"
#include <cmath>

Circle::Circle(double radius) : radius_(radius) {}

double Circle::area() const {
    return M_PI * radius_ * radius_;
}
