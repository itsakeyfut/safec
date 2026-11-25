#define STR(x) #x
#define ONCE_OF(x) _Pragma(STR(x))
ONCE_OF(once)
int once(void) { return 0; }
