void *malloc(int n);
int main(void) {
    int *p = malloc(8);
    if (p == 0) {
        return 1;
    }
    p[1] = 7;
    return 0;
}
