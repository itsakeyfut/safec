void *malloc(int n);

int main(void) {
    int *p = malloc(4);
    int **box = malloc(8);
    if (p == 0) {
        return 1;
    }
    if (box == 0) {
        return 1;
    }
    *box = p + 5;
    return 0;
}
