// Standalone external-DLL evidence; never used by the pinned the game broker.
// @category ExternalWindowsEvidence
import com.google.gson.*;
import ghidra.app.script.GhidraScript;
import ghidra.app.decompiler.*;
import ghidra.framework.Application;
import ghidra.program.database.mem.FileBytes;
import ghidra.program.model.address.*;
import ghidra.program.model.listing.*;
import ghidra.program.model.symbol.Reference;
import java.nio.file.*;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.*;

public class ExternalModuleEvidence extends GhidraScript {
    private final Gson gson = new GsonBuilder().setPrettyPrinting().create();
    private JsonObject request;
    private Path output;
    private final JsonArray artifacts = new JsonArray();

    private static void require(boolean ok, String why) {
        if (!ok) throw new IllegalStateException(why);
    }
    private static String hex(byte[] b) { return HexFormat.of().formatHex(b); }
    private static String hash(byte[] b) throws Exception {
        return hex(MessageDigest.getInstance("SHA-256").digest(b));
    }
    private static String hashFile(Path p) throws Exception { return hash(Files.readAllBytes(p)); }
    private static long number(JsonObject o, String k) { return o.get(k).getAsLong(); }
    private static String string(JsonObject o, String k) { return o.get(k).getAsString(); }
    private Address address(long rva) { return currentProgram.getImageBase().add(rva); }
    private String fileBytesHash(FileBytes file, boolean modified) throws Exception {
        MessageDigest d = MessageDigest.getInstance("SHA-256");
        byte[] b = new byte[65536];
        for (long offset=0; offset<file.getSize();) {
            monitor.checkCancelled();
            int count=(int)Math.min(b.length,file.getSize()-offset);
            int got=modified ? file.getModifiedBytes(offset,b,0,count) : file.getOriginalBytes(offset,b,0,count);
            require(got==count,"short imported FileBytes read");
            d.update(b,0,count); offset+=count;
        }
        return hex(d.digest());
    }
    private JsonObject identity() throws Exception {
        JsonObject id=request.getAsJsonObject("identity");
        Path snapshot=Path.of(string(request,"snapshot")).toRealPath();
        String expected=string(id,"sha256");
        require(Files.size(snapshot)==number(id,"bytes"),"snapshot size mismatch");
        require(hashFile(snapshot).equals(expected),"snapshot hash mismatch");
        require(currentProgram.getName().equals(string(id,"name")),"program name mismatch");
        require(snapshot.getFileName().toString().equals(string(id,"name")),"snapshot name mismatch");
        String importedPath=currentProgram.getExecutablePath();
        // Ghidra's PE loader uses /C:/... on Windows, not a Windows Path spelling.
        if (importedPath.matches("^/[a-zA-Z]:/.*")) importedPath=importedPath.substring(1);
        require(Path.of(importedPath).toRealPath().equals(snapshot),"import source path mismatch");
        require(expected.equalsIgnoreCase(currentProgram.getExecutableSHA256()),"program SHA-256 mismatch");
        require(currentProgram.getImageBase().getOffset()==number(id,"image_base"),"program image base mismatch");
        require(currentProgram.getLanguageID().toString().equals("x86:LE:64:default"),"program language mismatch");
        require(currentProgram.getExecutableFormat().equals("Portable Executable (PE)"),"program format mismatch");
        List<FileBytes> all=currentProgram.getMemory().getAllFileBytes();
        require(all.size()==1,"expected one imported file");
        FileBytes file=all.get(0);
        require(file.getSize()==number(id,"bytes") && file.getFileOffset()==0,"imported file extent mismatch");
        require(fileBytesHash(file,false).equals(expected),"original imported bytes mismatch");
        require(fileBytesHash(file,true).equals(expected),"modified imported bytes mismatch");
        JsonObject got=id.deepCopy();
        got.addProperty("original_import_sha256",expected);
        got.addProperty("modified_import_sha256",expected);
        got.addProperty("language",currentProgram.getLanguageID().toString());
        got.addProperty("format",currentProgram.getExecutableFormat());
        return got;
    }
    private void artifact(String name, byte[] bytes) throws Exception {
        require(bytes.length<=2*1024*1024,"artifact too large");
        Path file=output.resolve(name);
        Files.write(file,bytes,StandardOpenOption.CREATE_NEW);
        JsonObject row=new JsonObject();
        row.addProperty("path",name); row.addProperty("bytes",bytes.length); row.addProperty("sha256",hash(bytes));
        artifacts.add(row);
    }
    private void textArtifact(String name,String content) throws Exception {
        artifact(name,content.getBytes(StandardCharsets.UTF_8));
    }
    private JsonObject function(long rva, DecompInterface decompiler) throws Exception {
        Function f=getFunctionAt(address(rva));
        require(f!=null && !f.isExternal(),"exact function entry unavailable: "+Long.toHexString(rva));
        require(f.getBody().getNumAddresses()>0 && f.getBody().getNumAddresses()<=65536,"function body bound exceeded");
        JsonObject row=new JsonObject();
        row.addProperty("entry_rva",rva); row.addProperty("name",f.getName(true));
        JsonArray ranges=new JsonArray();
        AddressRangeIterator ri=f.getBody().getAddressRanges();
        while(ri.hasNext()) {
            AddressRange range=ri.next();
            long start=range.getMinAddress().subtract(currentProgram.getImageBase());
            require(start>=0 && start+range.getLength()<=number(request.getAsJsonObject("identity"),"size_of_image"),"body outside image");
            JsonArray pair=new JsonArray();pair.add(start);pair.add(range.getLength());ranges.add(pair);
        }
        row.add("body_ranges",ranges);
        JsonArray insns=new JsonArray();
        StringBuilder listing=new StringBuilder();
        InstructionIterator it=currentProgram.getListing().getInstructions(f.getBody(),true);
        while(it.hasNext()) {
            monitor.checkCancelled();
            require(insns.size()<8192,"instruction bound exceeded");
            Instruction ins=it.next();
            JsonObject item=new JsonObject();
            long irva=ins.getAddress().subtract(currentProgram.getImageBase());
            item.addProperty("rva",irva);item.addProperty("bytes",hex(ins.getBytes()));item.addProperty("text",ins.toString());
            JsonArray refs=new JsonArray();
            for(Reference ref:ins.getReferencesFrom()) {
                JsonObject rr=new JsonObject();
                rr.addProperty("to",ref.getToAddress().toString());rr.addProperty("type",ref.getReferenceType().toString());refs.add(rr);
            }
            item.add("references",refs);insns.add(item);
            listing.append(String.format("%016x  %-30s %s%n",ins.getAddress().getOffset(),hex(ins.getBytes()),ins));
        }
        require(insns.size()>0,"no instructions for seed");
        DecompileResults result=decompiler.decompileFunction(f,120,monitor);
        require(result!=null && result.decompileCompleted() && result.getDecompiledFunction()!=null,"decompile failed");
        String prefix="func_"+Long.toHexString(rva);
        String header="// module="+string(request.getAsJsonObject("identity"),"name")+
            "\n// sha256="+string(request.getAsJsonObject("identity"),"sha256")+
            "\n// image_base=0x"+Long.toHexString(currentProgram.getImageBase().getOffset())+
            "\n// entry_rva=0x"+Long.toHexString(rva)+"\n";
        textArtifact(prefix+".decompile.c",header+result.getDecompiledFunction().getC());
        textArtifact(prefix+".instructions.txt",header+listing);
        textArtifact(prefix+".instructions.json",gson.toJson(insns)+"\n");
        return row;
    }
    @Override public void run() throws Exception {
        String[] args=getScriptArgs();
        require(args.length==1,"expected request JSON only");
        Path requestPath=Path.of(args[0]).toRealPath();
        request=JsonParser.parseString(Files.readString(requestPath)).getAsJsonObject();
        require(string(request,"schema").equals("external-windows-request-v1"),"request schema mismatch");
        require(number(request,"depth")==0,"only depth zero is supported");
        String mode=string(request,"mode");
        require(Set.of("probe","functions","data").contains(mode),"unknown mode");
        JsonArray targets=request.getAsJsonArray("targets");
        require(targets.size()<=(mode.equals("data")?4:3),"target count exceeded");
        require(mode.equals("probe") ? targets.size()==0 : targets.size()>0,"wrong target count");
        long imageSize=number(request.getAsJsonObject("identity"),"size_of_image");
        Set<Long> unique=new HashSet<>();
        for(JsonElement target:targets) {
            long rva=number(target.getAsJsonObject(),"rva");
            require(rva>=0 && rva<imageSize && unique.add(rva),"invalid or duplicate RVA");
            if(mode.equals("data")) {
                long size=number(target.getAsJsonObject(),"size");
                require(size>0 && size<=512 && rva+size<=imageSize,"data extent exceeded");
            }
        }
        JsonObject got=identity(); // no output before exact identity succeeds
        output=Path.of(string(request,"output")).toAbsolutePath().normalize();
        Path job=requestPath.getParent().toRealPath();
        require(output.equals(job.resolve("evidence")),"output must be job/evidence");
        require(!Files.exists(output),"output already exists");
        Files.createDirectory(output);
        JsonArray functions=new JsonArray();
        if(mode.equals("functions")) {
            DecompInterface decompiler=new DecompInterface();
            require(decompiler.openProgram(currentProgram),"decompiler open failed");
            try { for(JsonElement target:targets) functions.add(function(number(target.getAsJsonObject(),"rva"),decompiler)); }
            finally { decompiler.dispose(); }
        } else if(mode.equals("data")) {
            for(JsonElement target:targets) {
                long rva=number(target.getAsJsonObject(),"rva");
                int size=(int)number(target.getAsJsonObject(),"size");
                byte[] b=new byte[size];
                require(currentProgram.getMemory().getBytes(address(rva),b)==size,"short data read");
                artifact("data_"+Long.toHexString(rva)+".bin",b);
            }
        }
        identity(); // detect snapshot/import identity changes during operation
        JsonObject manifest=new JsonObject();
        manifest.addProperty("schema","external-windows-evidence-v1");
        manifest.addProperty("request_sha256",hashFile(requestPath));
        manifest.addProperty("ghidra_version",Application.getApplicationVersion());
        manifest.addProperty("mode",mode);manifest.addProperty("depth",0);
        manifest.add("identity",got);manifest.add("targets",targets);manifest.add("functions",functions);manifest.add("artifacts",artifacts);
        Files.writeString(output.resolve("manifest.json"),gson.toJson(manifest)+"\n",StandardCharsets.UTF_8,StandardOpenOption.CREATE_NEW);
        println("EXTERNAL_WINDOWS_EVIDENCE_COMPLETE "+output);
    }
}
