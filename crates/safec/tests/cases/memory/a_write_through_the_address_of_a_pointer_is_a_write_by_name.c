void *malloc(int n);
void free(void *p);
void h(int v, int *r);
int main(void) {
    int x = 0;
    int *q = malloc(4);
    if (!q) return 0;
    free(q);
    h(*q, *&q = &x);
    return 0;
}
