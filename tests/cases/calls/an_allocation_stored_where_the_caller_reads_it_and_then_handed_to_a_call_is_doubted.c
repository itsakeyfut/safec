void *malloc(int n);
void release(int *p);
void put(int ** _Nonnull pp) {
    int *x = malloc(4);
    if (x == 0) {
        return;
    }
    *pp = x;
    release(x);
}
int main(void) {
    int *m[1];
    put(m);
    int *q = m[0];
    if (q == 0) {
        return 1;
    }
    return *q;
}
