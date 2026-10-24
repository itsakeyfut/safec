#include "a_function_declared_in_an_included_header_is_called.h"

int main(void) { return twice(2); }

int twice(int x) { return x + x; }
