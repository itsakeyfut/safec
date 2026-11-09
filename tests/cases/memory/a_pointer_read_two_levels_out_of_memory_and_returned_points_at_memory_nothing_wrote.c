void *malloc(int n);
int **f(void) { int ****bb = malloc(8); if (!bb) return 0; int ***box = malloc(8); if (!box) return 0; *bb = box; int **b = malloc(8); *box = b; return **bb; }
