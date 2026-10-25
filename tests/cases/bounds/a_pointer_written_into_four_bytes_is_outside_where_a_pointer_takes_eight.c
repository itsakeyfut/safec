void *malloc(int n);

int main(void) {
    int **pp = malloc(4);
    if (pp == 0) {
        return 1;
    }
    *pp = 0;
    return 0;
}
