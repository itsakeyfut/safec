void h(int v, int *r);
int main(void) {
    int x;
    int *q = 0;
    h(*q, q = &x);
    return 0;
}
