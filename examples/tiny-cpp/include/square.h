#pragma once

#include "shape.h"

class Square : public Shape {
public:
    explicit Square(double side);
    double area() const override;

private:
    double side_;
};
