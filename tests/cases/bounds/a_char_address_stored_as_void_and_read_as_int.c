void *malloc(int n);

int main(void) {
    char c = 0;
    void **box = malloc(8);
    if (box == 0) {
        return 1;
    }
    *box = &c;
    int *q = *box;
    if (q == 0) {
        return 1;
    }
    *q = 1;
    return 0;
}
