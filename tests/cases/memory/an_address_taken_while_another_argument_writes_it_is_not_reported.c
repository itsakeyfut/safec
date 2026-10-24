void g(int **pp, int *r);
int main(void) {
    int x;
    int *q = 0;
    g(&q, q = &x);
    return 0;
}
