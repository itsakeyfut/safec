#include "a_header_with_an_include_guard_included_twice_declares_once.h"
#include "a_header_with_an_include_guard_included_twice_declares_once.h"

int main(void) { struct S s; s.x = 0; return s.x; }
