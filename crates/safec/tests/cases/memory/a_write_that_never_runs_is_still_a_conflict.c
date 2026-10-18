void g(int *p, int r);
int main(void) {
    int x;
    int *q = &x;
    g(q, 0 && (q = &x));
    return 0;
}
