void *malloc(int n);
void free(void *p);

void reset(int **pp) {
    if (pp == 0) {
        return;
    }
    *pp = 0;
}

int main(void) {
    int *a = malloc(4);
    if (a == 0) {
        return 0;
    }
    free(a);
    reset(&a);
    return 0;
}
