void *malloc(int n);
void free(void *p);
int *give(int **pp);

int main(void) {
    int *p = malloc(4);
    if (p == 0) {
        return 0;
    }
    int *q = give(&p);
    if (q != 0) {
        return *q;
    }
    return 0;
}
