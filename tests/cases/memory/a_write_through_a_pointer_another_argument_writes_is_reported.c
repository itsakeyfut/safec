void h(int *p, int v);
int main(void) {
    int x;
    int *q = 0;
    h(q = &x, *q = 1);
    return 0;
}
