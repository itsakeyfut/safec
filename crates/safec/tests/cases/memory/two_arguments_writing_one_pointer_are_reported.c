void g(int *p, int *r);
int main(void) {
    int x;
    int *q = 0;
    g(q = 0, q = &x);
    return 0;
}
