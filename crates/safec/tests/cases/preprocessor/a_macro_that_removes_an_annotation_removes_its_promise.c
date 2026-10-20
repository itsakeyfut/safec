#define _Nonnull

int * _Nonnull g(void);

int f(void) {
    int *p = g();
    return *p;
}
