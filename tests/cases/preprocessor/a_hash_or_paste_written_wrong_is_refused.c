#define NOPARAM(x) # y
#define FIRST(x) ## x
#define LAST(x) x ##
#define OBJ ## a
#define P(a, b) a ## b
#define HASH_OBJ # a
#define TWICE(a, b) a ## ## b
#define BAD + ## -
int v = P(+, -);
int w = P(@, @);
int b = BAD;
int main(void) { return 0; }
