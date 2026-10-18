void g(int * _Nonnull p, int *r);
int main(void) {
    int x;
    int *q = 0;
    g(q, q = &x);
    return 0;
}
