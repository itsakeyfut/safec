void *malloc(int n);
void *memcpy(void *d, void *s, int n);

void use(int **pi) {
    if (pi != 0) {
        int *p = *pi;
        if (p != 0) {
            *p = 1;
        }
    }
}

void f(void **m) {
    if (m == 0) {
        return;
    }
    void **b = malloc(8);
    if (b == 0) {
        return;
    }
    memcpy(b, m, 8);
    void *pb = b;
    use(pb);
}

int main(void) {
    char c;
    void *v = &c;
    f(&v);
    return 0;
}
