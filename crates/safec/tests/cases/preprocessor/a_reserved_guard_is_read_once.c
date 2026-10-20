#ifndef _MAIN_C
#define _MAIN_C 1
#include "a_reserved_guard_is_read_once.h"
#include "a_reserved_guard_is_read_once.h"

int main(void) { struct S s; s.x = 0; return s.x; }
#endif
