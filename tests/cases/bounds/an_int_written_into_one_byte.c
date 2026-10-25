void *malloc(int n);

int main(void) {
    int *p = malloc(1);
    if (p == 0) {
        return 1;
    }
    *p = 5;
    return 0;
}
