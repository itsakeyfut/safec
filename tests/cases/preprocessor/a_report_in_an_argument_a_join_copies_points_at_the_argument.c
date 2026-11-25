void free(void *p);
#define C(a, b) a ## b

int f(int *q, int x) {
    free(q);
    return C(1 + *q + x, );
}
