void *malloc(int n);

int main(void) {
    char *p = malloc(0);
    if (p == 0) {
        return 1;
    }
    *p = 5;
    return 0;
}
