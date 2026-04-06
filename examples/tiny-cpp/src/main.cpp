#include "circle.h"
#include "square.h"
#include <iostream>
#include <memory>
#include <vector>

int main() {
    std::vector<std::unique_ptr<Shape>> shapes;
    shapes.push_back(std::make_unique<Circle>(5.0));
    shapes.push_back(std::make_unique<Square>(4.0));

    for (const auto& shape : shapes) {
        std::cout << "Area: " << shape->area() << std::endl;
    }

    return 0;
}
