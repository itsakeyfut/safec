void free(void *p);
#define RELEASE free(p)

void f(int * _Nonnull p) {
    RELEASE;
    RELEASE;
}
