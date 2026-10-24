#include "a_header_included_twice_is_read_twice.h"
#include "a_header_included_twice_is_read_twice.h"

int main(void) { return f(); }

int f(void) { return 0; }
