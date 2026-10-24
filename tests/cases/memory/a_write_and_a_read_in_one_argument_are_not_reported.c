void g(int *p, int *r);
int main(void) {
    int x;
    int *q = 0;
    g((q = &x, q), 0);
    return 0;
}
