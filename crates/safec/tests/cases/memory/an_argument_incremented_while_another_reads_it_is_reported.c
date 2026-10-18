void *malloc(int n);
void g(int *p, int *r);
int main(void) {
    int *q = malloc(8);
    if (!q) return 0;
    g(q, q++);
    return 0;
}
