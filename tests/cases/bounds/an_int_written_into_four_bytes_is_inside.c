void *malloc(int n);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 1;
    }
    *p = 5;
    return *p;
}
