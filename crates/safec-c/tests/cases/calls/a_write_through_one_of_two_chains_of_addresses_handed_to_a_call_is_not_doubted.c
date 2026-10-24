void *malloc(int n); void free(void *p); void two(int ***x, int ***y);
int f(void){
  int *a = malloc(4); if(!a) return 0;
  int *b = malloc(4); if(!b) { free(a); return 0; }
  int *q = malloc(4); if(!q) { free(a); free(b); return 0; }
  int **pa = &a; int **pb = &b;
  two(&pa, &pb);
  *pa = q;
  *q = 1;
  free(q);
  return 0;
}
