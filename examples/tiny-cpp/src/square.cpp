#include "square.h"

Square::Square(double side) : side_(side) {}

double Square::area() const {
    return side_ * side_;
}
