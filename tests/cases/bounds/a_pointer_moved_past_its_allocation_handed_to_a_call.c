void g(int *q) {
    if (q != 0) {
        *q = 1;
    }
}

void *malloc(int n);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    g(p + 5);
    return 0;
}
