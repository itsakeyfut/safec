void free(void *p);
#define CALL(e) (e)

void f(int * _Nonnull p) {
    CALL(free(p));
    CALL(free(p));
    CALL(free  (p));
}
