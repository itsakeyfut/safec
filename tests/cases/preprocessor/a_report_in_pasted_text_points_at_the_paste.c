void free(void *p);
#define CALL(f, p) (f ## ree(p))

void g(int * _Nonnull q) {
    CALL(f, q);
    CALL(f, q);
}
